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

    // 5. Parse project name from Cargo.toml
    let cargo_toml_path = build_dir.join("Cargo.toml");
    if !cargo_toml_path.exists() {
        return fail_job("Cargo.toml not found in project repository root");
    }

    let package_name = match std::fs::read_to_string(&cargo_toml_path) {
        Ok(content) => {
            let value: toml::Value = toml::from_str(&content)?;
            value
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        }
        Err(_) => None,
    };

    let package_name = match package_name {
        Some(name) => name,
        None => return fail_job("Failed to parse package name from Cargo.toml"),
    };

    // 6. Run Cargo Build compilation
    let mut cargo_args = vec!["build", "--release"];
    
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
        // 7. Compress built binary into tar.gz
        let binary_path = build_dir.join("target").join("release").join(&package_name);
        if !binary_path.exists() {
            return fail_job(&format!(
                "Cargo succeeded but binary not found at target/release/{}",
                package_name
            ));
        }

        let archive_filename = format!("{}.tar.gz", job.id);
        let archive_path = artifacts_dir.join(&archive_filename);

        // Compress using local tar utility inside Docker/Linux system
        let tar_status = Command::new("tar")
            .args([
                "-czf",
                &archive_path.to_string_lossy(),
                "-C",
                &build_dir.join("target").join("release").to_string_lossy(),
                &package_name,
            ])
            .status();

        match tar_status {
            Ok(status) if status.success() => {}
            _ => return fail_job("Failed to compress and package target compilation binary"),
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
