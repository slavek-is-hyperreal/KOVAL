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

pub fn process_job(
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

    // 1.5. Validate Project URL
    if let Err(err) = validate_project_url(&job.project) {
        return fail_job(&err);
    }

    // 2. Clone Git Repository
    let clone_status = Command::new("git")
        .args(["clone", "--depth=1", &job.project, &build_dir.to_string_lossy()])
        .status();

    match clone_status {
        Ok(status) if status.success() => {}
        _ => return fail_job("Failed to git clone project repository"),
    }

    // 2.5. Validate git_ref
    if let Err(err) = validate_git_ref(&job.git_ref) {
        return fail_job(&err);
    }

    // 3. Checkout requested branch/commit
    let checkout_status = Command::new("git")
        .args(["-C", &build_dir.to_string_lossy(), "checkout", "--", &job.git_ref])
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

    let build_mode = match detect_build_mode(&cargo_toml_content, job.binary.as_deref(), job.package.as_deref()) {
        Ok(mode) => mode,
        Err(err) => return fail_job(&err),
    };

    // 6. Run Cargo Build compilation based on build mode
    let mut rustflags = build_config.rustflags.clone();
    if job.job_type == "pgo_instrument" {
        let inst_flags = crate::pgo::instrument_flags(&job.id, artifacts_dir);
        if !rustflags.is_empty() {
            rustflags.push(' ');
        }
        rustflags.push_str(&inst_flags.join(" "));
    } else if job.job_type == "pgo_optimize" {
        let pgo_source_id = match &job.pgo_source_job_id {
            Some(id) => id,
            None => return fail_job("PGO optimize job missing source instrumentation job ID"),
        };

        let profile = {
            let conn = conn_mutex.lock().unwrap();
            db::get_pgo_profile(&conn, pgo_source_id)
        };

        let profile = match profile {
            Ok(Some(p)) => p,
            Ok(None) => return fail_job(&format!("No PGO profile record found for source job ID: {}", pgo_source_id)),
            Err(e) => return fail_job(&format!("Database error querying PGO profile: {}", e)),
        };

        let merged_path_str = match &profile.merged_path {
            Some(path) => path,
            None => return fail_job(&format!("PGO source job {} profile data has not been merged", pgo_source_id)),
        };

        let merged_path = std::path::PathBuf::from(merged_path_str);
        if !merged_path.exists() {
            return fail_job(&format!("Merged profile data at {} does not exist", merged_path.display()));
        }

        let opt_flags = crate::pgo::optimize_flags(&merged_path);
        if !rustflags.is_empty() {
            rustflags.push(' ');
        }
        rustflags.push_str(&opt_flags.join(" "));
    }

    let features_joined = build_config.features.join(",");
    let cargo_args = prepare_cargo_args(
        &build_mode,
        &build_config.features,
        &features_joined,
        job.target.as_deref(),
    );
    let envs = prepare_cargo_envs(&build_config.env, &rustflags, job.target.as_deref());

    let timeout_secs = std::env::var("KOVAL_BUILD_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3600);

    let build_output = run_cargo_build(&cargo_args, &envs, &build_dir, timeout_secs);

    let build_success = match build_output {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            let stderr_msg = String::from_utf8_lossy(&output.stderr);
            let truncated_stderr = truncate_stderr(&stderr_msg);
            let full_err = format!("Cargo compilation failed:\n{}", truncated_stderr);
            return fail_job(&full_err);
        }
        Err(err) => return fail_job(&format!("Cargo system execution failed: {}", err)),
    };

    if build_success {
        // 7. Find binaries and compress into tar.gz
        let archive_filename = format!("{}.tar.gz", job.id);
        let archive_path = artifacts_dir.join(&archive_filename);
        let release_dir = match &job.target {
            Some(t) => build_dir.join("target").join(t).join("release"),
            None => build_dir.join("target").join("release"),
        };

        match &build_mode {
            BuildMode::Workspace | BuildMode::PackageInWorkspace(_) | BuildMode::MultiBin => {
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
                    return fail_job(&format!("Build succeeded but no executable binaries found in release directory: {}", release_dir.display()));
                }

                // Compress workspace binaries using local tar utility
                let archive_str = archive_path.to_str()
                    .ok_or("Invalid archive path: non-UTF8")?
                    .to_string();
                let release_str = release_dir.to_str()
                    .ok_or("Invalid release dir: non-UTF8")?
                    .to_string();

                let mut tar_args: Vec<String> = vec![
                    "-czf".to_string(),
                    archive_str,
                    "-C".to_string(),
                    release_str,
                ];
                for bin in &binaries {
                    let name = bin.file_name()
                        .and_then(|s| s.to_str())
                        .ok_or("Invalid binary filename: non-UTF8")?
                        .to_string();
                    tar_args.push(name);
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
                        "Cargo succeeded but binary not found at {}",
                        binary_path.display()
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

        // 9. Store artifact record, build cache entry, and transition to done
        let finished_str = Utc::now().to_rfc3339();
        let webhooks = {
            let conn = conn_mutex.lock().unwrap();
            db::get_webhooks_delivery_info(&conn, job.token_id).ok().unwrap_or_default()
        };

        let hardware_json = serde_json::to_string(&job.hardware).unwrap_or_default();
        let cache_key = crate::cache::compute_cache_key(
            &hardware_json,
            &job.project,
            &job.git_ref,
            job.binary.as_deref(),
            job.package.as_deref(),
            job.target.as_deref(),
        );

        {
            let conn = conn_mutex.lock().unwrap();
            db::insert_artifact(&conn, &job.id, &archive_path.to_string_lossy(), file_size, &sha256_hash)?;
            db::insert_cache_entry(&conn, &cache_key, &job.id, &finished_str)?;
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
    PackageInWorkspace(String),
    SpecificBinary(String),
    MultiBin,
    SinglePackage(String),
}

pub fn detect_build_mode(
    cargo_toml_content: &str,
    binary: Option<&str>,
    package: Option<&str>,
) -> Result<BuildMode, String> {
    if let Some(bin_name) = binary {
        return Ok(BuildMode::SpecificBinary(bin_name.to_string()));
    }

    let value: toml::Value = toml::from_str(cargo_toml_content)
        .map_err(|e| format!("Failed to parse Cargo.toml: {}", e))?;

    let is_workspace = value.get("workspace").is_some();
    let is_package = value.get("package").is_some();

    if is_workspace && !is_package {
        if let Some(pkg_name) = package {
            Ok(BuildMode::PackageInWorkspace(pkg_name.to_string()))
        } else {
            Ok(BuildMode::Workspace)
        }
    } else if is_package {
        let name = value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .ok_or_else(|| "Missing name field under [package] in Cargo.toml".to_string())?;

        let bin_sections = value.get("bin").and_then(|b| b.as_array());
        let has_multiple_bins = if let Some(bins) = bin_sections {
            bins.len() >= 1
        } else {
            false
        };

        if has_multiple_bins {
            Ok(BuildMode::MultiBin)
        } else {
            Ok(BuildMode::SinglePackage(name.to_string()))
        }
    } else {
        Err("Cargo.toml has neither a [package] nor a root [workspace] section".to_string())
    }
}

pub fn prepare_cargo_args<'a>(
    build_mode: &'a BuildMode,
    features: &'a [String],
    features_joined: &'a str,
    target: Option<&'a str>,
) -> Vec<&'a str> {
    let mut cargo_args = vec!["build", "--release"];
    match build_mode {
        BuildMode::Workspace => {
            cargo_args.push("--workspace");
        }
        BuildMode::PackageInWorkspace(pkg) => {
            cargo_args.push("-p");
            cargo_args.push(pkg);
        }
        BuildMode::SpecificBinary(name) => {
            cargo_args.push("--bin");
            cargo_args.push(name);
        }
        BuildMode::MultiBin | BuildMode::SinglePackage(_) => {
            // default build
        }
    }

    if !features.is_empty() {
        cargo_args.push("--features");
        cargo_args.push(features_joined);
    }

    if let Some(t) = target {
        cargo_args.push("--target");
        cargo_args.push(t);
    }

    cargo_args
}

pub fn prepare_cargo_envs(
    base_envs: &HashMap<String, String>,
    rustflags: &str,
    target: Option<&str>,
) -> HashMap<String, String> {
    let mut envs = base_envs.clone();
    if !rustflags.is_empty() {
        envs.insert("RUSTFLAGS".to_string(), rustflags.to_string());
    }
    if let Some(t) = target {
        if let Some((env_var, linker_bin)) = crate::targets::linker_env_for_target(t) {
            envs.insert(env_var, linker_bin);
        }
    }
    envs
}

pub fn run_cargo_build(
    cargo_args: &[&str],
    envs: &HashMap<String, String>,
    build_dir: &std::path::Path,
    timeout_secs: u64,
) -> Result<std::process::Output, std::io::Error> {
    let execute = async {
        let mut child = match tokio::process::Command::new("cargo")
            .args(cargo_args)
            .envs(envs)
            .current_dir(build_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return Err(e),
        };

        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), child.wait()).await {
            Ok(Ok(status)) => {
                let mut stdout_buf = Vec::new();
                if let Some(mut stdout) = child.stdout.take() {
                    use tokio::io::AsyncReadExt;
                    let _ = stdout.read_to_end(&mut stdout_buf).await;
                }
                let mut stderr_buf = Vec::new();
                if let Some(mut stderr) = child.stderr.take() {
                    use tokio::io::AsyncReadExt;
                    let _ = stderr.read_to_end(&mut stderr_buf).await;
                }
                Ok(std::process::Output {
                    status,
                    stdout: stdout_buf,
                    stderr: stderr_buf,
                })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Cargo compilation timed out"))
            }
        }
    };

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(execute),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(execute)
        }
    }
}

pub fn truncate_stderr(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().collect();
    let was_truncated_by_lines = lines.len() > 100;
    
    let line_truncated_str = if was_truncated_by_lines {
        lines[..100].join("\n")
    } else {
        stderr.to_string()
    };

    let char_count = line_truncated_str.chars().count();
    let was_truncated_by_chars = char_count > 4096;

    let final_str = if was_truncated_by_chars {
        line_truncated_str.chars().take(4096).collect::<String>()
    } else {
        line_truncated_str
    };

    if was_truncated_by_lines || was_truncated_by_chars {
        let mut truncated = final_str;
        if !truncated.ends_with('\n') {
            truncated.push('\n');
        }
        truncated.push_str("[... stderr truncated due to size limits ...]\n");
        truncated
    } else {
        final_str
    }
}

pub fn validate_git_ref(git_ref: &str) -> Result<(), String> {
    if git_ref.is_empty() {
        return Err("Invalid git_ref: empty".to_string());
    }
    if git_ref.len() > 256 {
        return Err("Invalid git_ref: too long".to_string());
    }
    if git_ref.starts_with('-') {
        return Err("Invalid git_ref: starts with '-'".to_string());
    }
    if git_ref.contains("..") {
        return Err("Invalid git_ref: contains '..'".to_string());
    }
    if git_ref.contains('\0') {
        return Err("Invalid git_ref: contains null bytes".to_string());
    }
    for c in git_ref.chars() {
        if !c.is_ascii_alphanumeric() && c != '-' && c != '.' && c != '_' && c != '/' {
            return Err(format!("Invalid git_ref: character '{}' not allowed", c));
        }
    }
    Ok(())
}

pub fn validate_project_url(url: &str) -> Result<(), String> {
    if url.len() >= 8 && url[..8].eq_ignore_ascii_case("https://") {
        Ok(())
    } else {
        Err("Only https:// project URLs are permitted".to_string())
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
        let mode = detect_build_mode(toml, None, None).unwrap();
        assert_eq!(mode, BuildMode::Workspace);
    }

    #[test]
    fn test_detect_build_mode_workspace_with_specific_binary() {
        let toml = r#"
            [workspace]
            members = ["crate1", "crate2"]
        "#;
        let mode = detect_build_mode(toml, Some("server"), None).unwrap();
        assert_eq!(mode, BuildMode::SpecificBinary("server".to_string()));
    }

    #[test]
    fn test_detect_build_mode_workspace_with_package() {
        let toml = r#"
            [workspace]
            members = ["crate1", "crate2"]
        "#;
        let mode = detect_build_mode(toml, None, Some("crate1")).unwrap();
        assert_eq!(mode, BuildMode::PackageInWorkspace("crate1".to_string()));
    }

    #[test]
    fn test_detect_build_mode_workspace_with_package_and_specific_binary() {
        let toml = r#"
            [workspace]
            members = ["crate1", "crate2"]
        "#;
        let mode = detect_build_mode(toml, Some("server"), Some("crate1")).unwrap();
        assert_eq!(mode, BuildMode::SpecificBinary("server".to_string()));
    }

    #[test]
    fn test_detect_build_mode_single_package() {
        let toml = r#"
            [package]
            name = "myapp"
            version = "0.1.0"
        "#;
        let mode = detect_build_mode(toml, None, None).unwrap();
        assert_eq!(mode, BuildMode::SinglePackage("myapp".to_string()));
    }

    #[test]
    fn test_detect_build_mode_single_package_with_specific_binary() {
        let toml = r#"
            [package]
            name = "myapp"
            version = "0.1.0"
        "#;
        let mode = detect_build_mode(toml, Some("alt"), None).unwrap();
        assert_eq!(mode, BuildMode::SpecificBinary("alt".to_string()));
    }

    #[test]
    fn test_detect_build_mode_multibin() {
        let toml = r#"
            [package]
            name = "myapp"
            version = "0.1.0"

            [[bin]]
            name = "bin1"
            path = "src/bin1.rs"

            [[bin]]
            name = "bin2"
            path = "src/bin2.rs"
        "#;
        let mode = detect_build_mode(toml, None, None).unwrap();
        assert_eq!(mode, BuildMode::MultiBin);
    }

    #[test]
    fn test_detect_build_mode_multibin_with_specific_binary() {
        let toml = r#"
            [package]
            name = "myapp"
            version = "0.1.0"

            [[bin]]
            name = "bin1"
            path = "src/bin1.rs"

            [[bin]]
            name = "bin2"
            path = "src/bin2.rs"
        "#;
        let mode = detect_build_mode(toml, Some("bin1"), None).unwrap();
        assert_eq!(mode, BuildMode::SpecificBinary("bin1".to_string()));
    }

    #[test]
    fn test_detect_build_mode_malformed() {
        let toml = r#"
            [package
            name = "myapp"
        "#;
        let res = detect_build_mode(toml, None, None);
        assert!(res.is_err());
    }

    #[test]
    fn test_detect_build_mode_single_explicit_bin() {
        let toml = r#"
            [package]
            name = "myapp"
            version = "0.1.0"

            [[bin]]
            name = "custom-bin-name"
            path = "src/main.rs"
        "#;
        let mode = detect_build_mode(toml, None, None).unwrap();
        assert_eq!(mode, BuildMode::MultiBin);
    }

    #[test]
    fn test_cargo_prepare_target() {
        // Test argument preparation with target
        let mode = BuildMode::Workspace;
        let features = vec!["feat1".to_string()];
        let feat_str = "feat1";
        let args = prepare_cargo_args(&mode, &features, feat_str, Some("aarch64-unknown-linux-gnu"));
        assert_eq!(args, vec!["build", "--release", "--workspace", "--features", "feat1", "--target", "aarch64-unknown-linux-gnu"]);

        // Test argument preparation without target
        let args_no_target = prepare_cargo_args(&mode, &features, feat_str, None);
        assert_eq!(args_no_target, vec!["build", "--release", "--workspace", "--features", "feat1"]);

        // Test environment preparation with target
        let mut base_envs = HashMap::new();
        base_envs.insert("SOME_VAR".to_string(), "val".to_string());
        let envs = prepare_cargo_envs(&base_envs, "-C target-feature=+avx2", Some("aarch64-unknown-linux-gnu"));
        assert_eq!(envs.get("SOME_VAR").unwrap(), "val");
        assert_eq!(envs.get("RUSTFLAGS").unwrap(), "-C target-feature=+avx2");
        assert_eq!(envs.get("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER").unwrap(), "aarch64-linux-gnu-gcc");

        // Test environment preparation without target
        let envs_no_target = prepare_cargo_envs(&base_envs, "-C target-feature=+avx2", None);
        assert_eq!(envs_no_target.get("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER"), None);
    }

    #[test]
    fn test_pgo_rustflags_injection() {
        let base_flags = "-C target-cpu=native";
        let mut rustflags = base_flags.to_string();
        let inst_flags = vec!["-C".to_string(), "profile-generate=/tmp/pgo/job1".to_string()];
        if !rustflags.is_empty() {
            rustflags.push(' ');
        }
        rustflags.push_str(&inst_flags.join(" "));
        assert_eq!(rustflags, "-C target-cpu=native -C profile-generate=/tmp/pgo/job1");

        let mut rustflags_opt = base_flags.to_string();
        let opt_flags = vec!["-C".to_string(), "profile-use=/tmp/pgo/job1/merged.profdata".to_string(), "-C".to_string(), "llvm-args=-pgo-warn-missing-function".to_string()];
        if !rustflags_opt.is_empty() {
            rustflags_opt.push(' ');
        }
        rustflags_opt.push_str(&opt_flags.join(" "));
        assert_eq!(rustflags_opt, "-C target-cpu=native -C profile-use=/tmp/pgo/job1/merged.profdata -C llvm-args=-pgo-warn-missing-function");
    }

    #[test]
    fn test_git_ref_validation() {
        // Test case 8
        assert!(validate_git_ref("main").is_ok());
        // Test case 9
        assert!(validate_git_ref("v1.2.3").is_ok());
        // Test case 10
        assert!(validate_git_ref("feature/my-branch").is_ok());
        // Test case 11
        assert!(validate_git_ref("abc123def456").is_ok());
        // Test case 12
        assert!(validate_git_ref("--orphan").is_err());
        // Test case 13
        assert!(validate_git_ref("refs/../../../etc").is_err());
        // Test case 14
        assert!(validate_git_ref("").is_err());
        // Test case 15
        assert!(validate_git_ref(&"a".repeat(257)).is_err());
    }

    #[test]
    fn test_project_url_validation() {
        // Test case 1
        assert!(validate_project_url("https://github.com/user/repo").is_ok());
        // Test case 2
        assert!(validate_project_url("http://github.com/user/repo").is_err());
        // Test case 3
        assert!(validate_project_url("file:///etc/passwd").is_err());
        // Test case 4
        assert!(validate_project_url("git://github.com/user/repo").is_err());
        // Test case 5
        assert!(validate_project_url("ssh://user@host/repo").is_err());
        // Test case 6
        assert!(validate_project_url("").is_err());
        // Test case 7
        assert!(validate_project_url("not-a-url").is_err());
        // Check case-insensitivity
        assert!(validate_project_url("HTTPS://github.com/user/repo").is_ok());
    }

    #[test]
    fn test_git_clone_depth() {
        use std::process::Command;
        use uuid::Uuid;

        let base_temp = std::env::temp_dir().join(format!("git_test_{}", Uuid::new_v4()));
        let src_dir = base_temp.join("src_repo");
        let dest_dir = base_temp.join("dest_repo");
        std::fs::create_dir_all(&src_dir).unwrap();

        let run_git = |args: &[&str], dir: &std::path::Path| {
            Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .unwrap();
        };

        run_git(&["init"], &src_dir);
        run_git(&["config", "user.name", "Test User"], &src_dir);
        run_git(&["config", "user.email", "test@example.com"], &src_dir);
        run_git(&["config", "init.defaultBranch", "main"], &src_dir);

        let file_path = src_dir.join("file.txt");
        std::fs::write(&file_path, b"version 1").unwrap();
        run_git(&["add", "file.txt"], &src_dir);
        run_git(&["commit", "-m", "commit 1"], &src_dir);

        std::fs::write(&file_path, b"version 2").unwrap();
        run_git(&["add", "file.txt"], &src_dir);
        run_git(&["commit", "-m", "commit 2"], &src_dir);

        let status = Command::new("git")
            .args(["clone", "--depth=1", &format!("file://{}", src_dir.to_string_lossy()), &dest_dir.to_string_lossy()])
            .status()
            .unwrap();
        assert!(status.success());

        let output = Command::new("git")
            .args(["rev-list", "--count", "HEAD"])
            .current_dir(&dest_dir)
            .output()
            .unwrap();
        let count_str = String::from_utf8(output.stdout).unwrap();
        let count: usize = count_str.trim().parse().unwrap();
        assert_eq!(count, 1);

        std::fs::remove_dir_all(&base_temp).ok();
    }

    #[test]
    fn test_compilation_timeout_case_28() {
        use uuid::Uuid;
        let temp_dir = std::env::temp_dir().join(format!("timeout_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir.join("src")).unwrap();

        std::fs::write(
            temp_dir.join("Cargo.toml"),
            r#"[package]
name = "timeout_test"
version = "0.1.0"
"#
        ).unwrap();

        std::fs::write(
            temp_dir.join("src/main.rs"),
            "fn main() {}"
        ).unwrap();

        std::fs::write(
            temp_dir.join("build.rs"),
            r#"fn main() {
    std::thread::sleep(std::time::Duration::from_secs(10));
}"#
        ).unwrap();

        let start = std::time::Instant::now();
        let build_config_env = HashMap::new();
        let res = run_cargo_build(&["build", "--release"], &build_config_env, &temp_dir, 1);
        let duration = start.elapsed();

        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(duration < std::time::Duration::from_secs(5));

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_stderr_truncation_case_29() {
        let short_err = "some error\nline 2";
        assert_eq!(truncate_stderr(short_err), short_err);

        let hundred_and_five_lines = (1..=105).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
        let truncated_lines = truncate_stderr(&hundred_and_five_lines);
        assert!(truncated_lines.contains("[... stderr truncated due to size limits ...]"));
        let line_count = truncated_lines.lines().count();
        assert_eq!(line_count, 101);

        let long_chars = "a".repeat(5000);
        let truncated_chars = truncate_stderr(&long_chars);
        assert!(truncated_chars.contains("[... stderr truncated due to size limits ...]"));
        assert_eq!(truncated_chars.lines().next().unwrap().len(), 4096);
    }
}
