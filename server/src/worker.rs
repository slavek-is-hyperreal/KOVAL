use chrono::Utc;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Receiver;

use crate::db;
use crate::forge::{self, KovalToml};
use crate::queue::Job;

pub struct BuildWorker {
    conn: Arc<Mutex<Connection>>,
    receiver: Receiver<Job>,
    artifacts_dir: PathBuf,
}

impl BuildWorker {
    pub fn new(
        conn: Arc<Mutex<Connection>>,
        receiver: Receiver<Job>,
        artifacts_dir: PathBuf,
    ) -> Self {
        // Ensure artifacts directory exists
        std::fs::create_dir_all(&artifacts_dir).ok();
        Self {
            conn,
            receiver,
            artifacts_dir,
        }
    }

    /// Starts the background build processing loop
    pub fn start(mut self) {
        tokio::spawn(async move {
            while let Some(job) = self.receiver.recv().await {
                let conn_clone = self.conn.clone();
                let artifacts_dir_clone = self.artifacts_dir.clone();
                
                // Process each job in a separate blocking task to ensure it doesn't block tokio executor threads
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = process_job(&conn_clone, job, &artifacts_dir_clone) {
                        eprintln!("Error processing build job: {:?}", e);
                    }
                });
            }
        });
    }
}

fn process_job(
    conn_mutex: &Arc<Mutex<Connection>>,
    job: Job,
    artifacts_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let now_str = Utc::now().to_rfc3339();

    // 1. Transition state in DB: building
    {
        let conn = conn_mutex.lock().unwrap();
        db::update_job_status(&conn, &job.id, "building", Some(&now_str), None, None)?;
    }

    // Define temporary build path
    let build_dir = std::env::temp_dir().join(format!("koval_build_{}", job.id));
    if build_dir.exists() {
        std::fs::remove_dir_all(&build_dir).ok();
    }

    // Helper closure to fail the job and update DB
    let fail_job = |err_msg: &str| -> Result<(), Box<dyn std::error::Error>> {
        let finished_str = Utc::now().to_rfc3339();
        let webhooks = {
            let conn = conn_mutex.lock().unwrap();
            db::get_webhooks_delivery_info(&conn, job.token_id).ok().unwrap_or_default()
        };

        {
            let conn = conn_mutex.lock().unwrap();
            db::update_job_status(&conn, &job.id, "failed", None, Some(&finished_str), Some(err_msg))?;
        }

        std::fs::remove_dir_all(&build_dir).ok();

        // Trigger Webhook Delivery
        let payload = schema::WebhookPayload {
            job_id: job.id.clone(),
            status: "failed".to_string(),
            finished_at: Some(finished_str),
            project: job.project.clone(),
            sha256: None,
        };

        if !webhooks.is_empty() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    crate::webhook::deliver(payload, webhooks).await;
                });
            } else {
                eprintln!("Failed to get Tokio handle for webhook delivery");
            }
        }

        Ok(())
    };

    // 2. Clone Git Repository
    let clone_status = Command::new("git")
        .args(["clone", &job.project, &build_dir.to_string_lossy()])
        .status();

    match clone_status {
        Ok(status) if status.success() => {}
        _ => return fail_job("Failed to git clone project repository"),
    }

    // 3. Checkout requested branch/commit
    let checkout_status = Command::new("git")
        .args(["-C", &build_dir.to_string_lossy(), "checkout", &job.git_ref])
        .status();

    match checkout_status {
        Ok(status) if status.success() => {}
        _ => return fail_job(&format!("Failed to checkout git ref: {}", job.git_ref)),
    }

    // 4. Parse koval.toml and evaluate Build Configuration
    let koval_toml_path = build_dir.join("koval.toml");
    let config = if koval_toml_path.exists() {
        match std::fs::read_to_string(&koval_toml_path) {
            Ok(content) => match toml::from_str::<KovalToml>(&content) {
                Ok(parsed) => parsed,
                Err(err) => return fail_job(&format!("Failed to parse koval.toml: {}", err)),
            },
            Err(err) => return fail_job(&format!("Failed to read koval.toml: {}", err)),
        }
    } else {
        KovalToml::default()
    };

    let build_config = forge::build_config(&job.hardware, &config);

    // 5. Parse and determine build mode
    let cargo_toml_path = build_dir.join("Cargo.toml");
    if !cargo_toml_path.exists() {
        return fail_job("Cargo.toml not found in project repository root");
    }

    let cargo_toml_content = match std::fs::read_to_string(&cargo_toml_path) {
        Ok(c) => c,
        Err(err) => return fail_job(&format!("Failed to read Cargo.toml: {}", err)),
    };

    let build_mode = match detect_build_mode(&cargo_toml_content, job.binary.as_deref()) {
        Ok(mode) => mode,
        Err(err) => return fail_job(&err),
    };

    // 6. Run Cargo Build compilation based on build mode
    let mut cargo_args = vec!["build", "--release"];
    match &build_mode {
        BuildMode::Workspace => {
            cargo_args.push("--workspace");
        }
        BuildMode::SpecificBinary(name) => {
            cargo_args.push("--bin");
            cargo_args.push(name);
        }
        BuildMode::SinglePackage(_) => {
            // default build
        }
    }

    // Add custom features if any are matched
    let features_joined = build_config.features.join(",");
    if !build_config.features.is_empty() {
        cargo_args.push("--features");
        cargo_args.push(&features_joined);
    }

    // Prepare cargo process environment
    let mut envs: HashMap<String, String> = build_config.env.clone();
    if !build_config.rustflags.is_empty() {
        envs.insert("RUSTFLAGS".to_string(), build_config.rustflags.clone());
    }

    let build_output = Command::new("cargo")
        .args(cargo_args)
        .envs(&envs)
        .current_dir(&build_dir)
        .output();

    let build_success = match build_output {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            let stderr_msg = String::from_utf8_lossy(&output.stderr);
            let full_err = format!("Cargo compilation failed:\n{}", stderr_msg);
            return fail_job(&full_err);
        }
        Err(err) => return fail_job(&format!("Cargo system execution failed: {}", err)),
    };

    if build_success {
        // 7. Find binaries and compress into tar.gz
        let archive_filename = format!("{}.tar.gz", job.id);
        let archive_path = artifacts_dir.join(&archive_filename);
        let release_dir = build_dir.join("target").join("release");

        match &build_mode {
            BuildMode::Workspace => {
                let mut binaries = Vec::new();
                let dir_entries = match std::fs::read_dir(&release_dir) {
                    Ok(entries) => entries,
                    Err(err) => return fail_job(&format!("Failed to read target/release/ directory: {}", err)),
                };

                for entry in dir_entries {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                            if file_name.starts_with('.') || file_name == "build" {
                                continue;
                            }
                            if path.extension().is_none() {
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    if let Ok(metadata) = path.metadata() {
                                        let mode = metadata.permissions().mode();
                                        if mode & 0o111 != 0 {
                                            binaries.push(path);
                                        }
                                    }
                                }
                                #[cfg(not(unix))]
                                {
                                    binaries.push(path);
                                }
                            }
                        }
                    }
                }

                if binaries.is_empty() {
                    return fail_job("Workspace build succeeded but no executable binaries found in target/release/");
                }

                // Compress workspace binaries using local tar utility
                let mut tar_args = vec![
                    "-czf",
                    archive_path.to_str().ok_or("Invalid archive path")?,
                    "-C",
                    release_dir.to_str().ok_or("Invalid release dir path")?,
                ];
                for bin in &binaries {
                    let file_name = bin.file_name().and_then(|s| s.to_str()).ok_or("Invalid binary name")?;
                    tar_args.push(file_name);
                }

                let tar_status = Command::new("tar")
                    .args(&tar_args)
                    .status();

                match tar_status {
                    Ok(status) if status.success() => {}
                    _ => return fail_job("Failed to compress and package target compilation binaries"),
                }
            }
            BuildMode::SpecificBinary(name) | BuildMode::SinglePackage(name) => {
                let binary_path = release_dir.join(name);
                if !binary_path.exists() {
                    return fail_job(&format!(
                        "Cargo succeeded but binary not found at target/release/{}",
                        name
                    ));
                }

                let tar_status = Command::new("tar")
                    .args([
                        "-czf",
                        &archive_path.to_string_lossy(),
                        "-C",
                        &release_dir.to_string_lossy(),
                        name,
                    ])
                    .status();

                match tar_status {
                    Ok(status) if status.success() => {}
                    _ => return fail_job("Failed to compress and package target compilation binary"),
                }
            }
        }

        // 8. Calculate SHA256 of the created artifact
        let mut file = File::open(&archive_path)?;
        let mut sha_hasher = Sha256::new();
        let mut buffer = [0; 4096];
        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            sha_hasher.update(&buffer[..bytes_read]);
        }
        let sha256_hash = format!("{:x}", sha_hasher.finalize());
        let file_size = archive_path.metadata()?.len();

        // 9. Store artifact record and transition to done
        let finished_str = Utc::now().to_rfc3339();
        let webhooks = {
            let conn = conn_mutex.lock().unwrap();
            db::get_webhooks_delivery_info(&conn, job.token_id).ok().unwrap_or_default()
        };

        {
            let conn = conn_mutex.lock().unwrap();
            db::insert_artifact(&conn, &job.id, &archive_path.to_string_lossy(), file_size, &sha256_hash)?;
            db::update_job_status(&conn, &job.id, "done", None, Some(&finished_str), None)?;
        }

        // Trigger Webhook Delivery
        let payload = schema::WebhookPayload {
            job_id: job.id.clone(),
            status: "done".to_string(),
            finished_at: Some(finished_str),
            project: job.project.clone(),
            sha256: Some(sha256_hash),
        };

        if !webhooks.is_empty() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    crate::webhook::deliver(payload, webhooks).await;
                });
            } else {
                eprintln!("Failed to get Tokio handle for webhook delivery");
            }
        }
    }

    // 10. Clean up workspace directory
    std::fs::remove_dir_all(&build_dir).ok();
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum BuildMode {
    Workspace,
    SpecificBinary(String),
    SinglePackage(String),
}

pub fn detect_build_mode(cargo_toml_content: &str, binary: Option<&str>) -> Result<BuildMode, String> {
    if let Some(bin_name) = binary {
        return Ok(BuildMode::SpecificBinary(bin_name.to_string()));
    }

    let value: toml::Value = toml::from_str(cargo_toml_content)
        .map_err(|e| format!("Failed to parse Cargo.toml: {}", e))?;

    let is_workspace = value.get("workspace").is_some();
    let is_package = value.get("package").is_some();

    if is_workspace && !is_package {
        Ok(BuildMode::Workspace)
    } else if is_package {
        let name = value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .ok_or_else(|| "Missing name field under [package] in Cargo.toml".to_string())?;
        Ok(BuildMode::SinglePackage(name.to_string()))
    } else {
        Err("Cargo.toml has neither a [package] nor a root [workspace] section".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_build_mode_workspace() {
        let toml = r#"
            [workspace]
            members = ["crate1", "crate2"]
        "#;
        let mode = detect_build_mode(toml, None).unwrap();
        assert_eq!(mode, BuildMode::Workspace);
    }

    #[test]
    fn test_detect_build_mode_workspace_with_specific_binary() {
        let toml = r#"
            [workspace]
            members = ["crate1", "crate2"]
        "#;
        let mode = detect_build_mode(toml, Some("server")).unwrap();
        assert_eq!(mode, BuildMode::SpecificBinary("server".to_string()));
    }

    #[test]
    fn test_detect_build_mode_single_package() {
        let toml = r#"
            [package]
            name = "myapp"
            version = "0.1.0"
        "#;
        let mode = detect_build_mode(toml, None).unwrap();
        assert_eq!(mode, BuildMode::SinglePackage("myapp".to_string()));
    }

    #[test]
    fn test_detect_build_mode_single_package_with_specific_binary() {
        let toml = r#"
            [package]
            name = "myapp"
            version = "0.1.0"
        "#;
        let mode = detect_build_mode(toml, Some("alt")).unwrap();
        assert_eq!(mode, BuildMode::SpecificBinary("alt".to_string()));
    }

    #[test]
    fn test_detect_build_mode_malformed() {
        let toml = r#"
            [package
            name = "myapp"
        "#;
        let res = detect_build_mode(toml, None);
        assert!(res.is_err());
    }
}
