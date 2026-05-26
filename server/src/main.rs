use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub mod auth;
pub mod db;
pub mod forge;
pub mod queue;
pub mod routes;
pub mod webhook;
pub mod worker;
pub mod cache;
pub mod targets;
pub mod pgo;

use crate::queue::JobQueue;
use crate::routes::AppState;
use crate::worker::BuildWorker;

pub fn bootstrap_admin_token(conn: &rusqlite::Connection) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let active_tokens = db::get_active_tokens(conn)?;
    if active_tokens.is_empty() {
        let (admin_token, is_generated) = match std::env::var("KOVAL_ADMIN_TOKEN") {
            Ok(token) => (token, false),
            Err(_) => (uuid::Uuid::new_v4().to_string(), true),
        };
        let hashed = auth::hash_token(&admin_token)?;
        db::insert_token(
            conn,
            &hashed,
            "Default Admin Token",
            &chrono::Utc::now().to_rfc3339(),
        )?;
        
        if is_generated {
            eprintln!("=======================================================");
            eprintln!("  WARNING: KOVAL_ADMIN_TOKEN is not set.");
            eprintln!("  A random bootstrap admin token has been generated:");
            eprintln!("  Bearer Token: {}", admin_token);
            eprintln!("  Please configure a secure KOVAL_ADMIN_TOKEN environment variable");
            eprintln!("  and rotate this token immediately in production.");
            eprintln!("=======================================================");
        } else {
            println!("=======================================================");
            println!("  BOOTSTRAPPED DEFAULT DEVELOPER ADMIN TOKEN FROM ENV:");
            println!("  Bearer Token: {}", admin_token);
            println!("=======================================================");
        }
        Ok(Some(admin_token))
    } else {
        Ok(None)
    }
}

#[tokio::main]
async fn main() {
    println!("Starting Koval Server...");

    // 1. Load configuration from environment variables
    let db_path = std::env::var("KOVAL_DB").unwrap_or_else(|_| "koval.db".to_string());
    let queue_capacity = std::env::var("KOVAL_QUEUE_CAPACITY")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10);
    let rate_limit = std::env::var("KOVAL_RATE_LIMIT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(20);
    let artifacts_dir = PathBuf::from(std::env::var("KOVAL_ARTIFACTS_DIR").unwrap_or_else(|_| "artifacts".to_string()));
    let port = std::env::var("KOVAL_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(8080);

    // 2. Initialize Persistent Database
    let conn = db::init_db(&db_path).expect("Failed to initialize database");
    
    // 3. Bootstrap default developer token if table is empty
    let _ = bootstrap_admin_token(&conn).expect("Failed to bootstrap default admin token");

    let shared_conn = Arc::new(Mutex::new(conn));

    // 4. Initialize Job Dispatcher Queue
    let (queue, receiver) = JobQueue::new(queue_capacity);
    let shared_queue = Arc::new(queue);

    // 5. Spawn background compilation worker loop
    let worker = BuildWorker::new(shared_conn.clone(), receiver, artifacts_dir.clone());
    worker.start();
    println!("Background build worker pipeline started successfully.");

    // 6. Build App State and Axum Router
    let state = AppState {
        conn: shared_conn,
        queue: shared_queue,
        artifacts_dir,
        rate_limit_limit: rate_limit,
    };

    let app = Router::new()
        .route("/build", post(routes::build::build_handler))
        .route("/build/:id/status", get(routes::status::status_handler))
        .route("/build/:id/binary", get(routes::binary::binary_handler))
        .route("/webhooks", post(routes::webhooks::register_webhook_handler).get(routes::webhooks::list_webhooks_handler))
        .route("/webhooks/:id", axum::routing::delete(routes::webhooks::delete_webhook_handler))
        .route("/tokens", post(routes::tokens::create_token_handler).get(routes::tokens::list_tokens_handler))
        .route("/tokens/:id", axum::routing::delete(routes::tokens::delete_token_handler))
        .route("/ui", get(routes::ui::ui_handler))
        .route("/jobs", get(routes::jobs::list_jobs_handler))
        .route("/install/:project", get(routes::install::install_script_handler))
        .route("/probe/static/:arch", get(routes::install::static_probe_handler))
        .route("/forge/install", post(routes::install::forge_install_handler))
        .route("/pgo/profiles/:instrument_job_id", post(routes::pgo::upload_profiles_handler))
        .route("/pgo/profiles/:instrument_job_id/merged.profdata", get(routes::pgo::get_merged_profile_handler))
        .layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024))
        .with_state(state);

    // 7. Bind and run Axum server
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|_| panic!("Failed to bind to port {}", port));

    println!("Koval API listening on http://0.0.0.0:{}...", port);
    axum::serve(listener, app).await.expect("Axum engine runtime failure");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use tower::util::ServiceExt; // for oneshot
    use serde_json::Value;

    fn build_test_router() -> (Router, Arc<Mutex<rusqlite::Connection>>, Arc<JobQueue>, tokio::sync::mpsc::Receiver<crate::queue::Job>) {
        let conn = db::init_db(":memory:").unwrap();
        
        // Setup a test token
        let token_hash = auth::hash_token("test_bearer").unwrap();
        db::insert_token(&conn, &token_hash, "Test Client", &chrono::Utc::now().to_rfc3339()).unwrap();

        let shared_conn = Arc::new(Mutex::new(conn));
        let (queue, rx) = JobQueue::new(5);
        let shared_queue = Arc::new(queue);
        let state = AppState {
            conn: shared_conn.clone(),
            queue: shared_queue.clone(),
            artifacts_dir: PathBuf::from("/tmp/artifacts"),
            rate_limit_limit: 10,
        };

        let app = Router::new()
            .route("/build", post(routes::build::build_handler))
            .route("/build/:id/status", get(routes::status::status_handler))
            .route("/install/:project", get(routes::install::install_script_handler))
            .route("/probe/static/:arch", get(routes::install::static_probe_handler))
            .route("/forge/install", post(routes::install::forge_install_handler))
            .route("/pgo/profiles/:instrument_job_id", post(routes::pgo::upload_profiles_handler))
            .route("/pgo/profiles/:instrument_job_id/merged.profdata", get(routes::pgo::get_merged_profile_handler))
            .layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024))
            .with_state(state);

        (app, shared_conn, shared_queue, rx)
    }

    #[tokio::test]
    async fn test_e2e_unauthorized_access() {
        let (app, _, _, _rx) = build_test_router();

        let payload = r#"{
            "project": "url",
            "git_ref": "main",
            "hardware": {
                "cpu": {
                    "flags": ["avx2"],
                    "cache_topology": "L1:32KB",
                    "core_count": 4
                },
                "memory": {
                    "total_bytes": 8589934592,
                    "available_bytes": 4294967296,
                    "bandwidth_mbs": 12000.0
                },
                "storage": {
                    "io_uring": false,
                    "o_direct": true,
                    "read_speed_mbs": 450.0,
                    "write_speed_mbs": 400.0
                },
                "gpu": {
                    "devices": []
                }
            }
        }"#;

        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_e2e_authorized_job_handling() {
        let (app, _conn, _, _rx) = build_test_router();

        let payload = r#"{
            "project": "https://github.com/example/lib",
            "git_ref": "v1.0",
            "hardware": {
                "cpu": {
                    "flags": ["avx2"],
                    "cache_topology": "L1:32KB",
                    "core_count": 4
                },
                "memory": {
                    "total_bytes": 8589934592,
                    "available_bytes": 4294967296,
                    "bandwidth_mbs": 12000.0
                },
                "storage": {
                    "io_uring": false,
                    "o_direct": true,
                    "read_speed_mbs": 450.0,
                    "write_speed_mbs": 400.0
                },
                "gpu": {
                    "devices": []
                }
            }
        }"#;

        // 1. Submit valid build request
        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        // Extract job ID from JSON response
        let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
        let job_id = body_json["id"].as_str().expect("Response should contain string ID");
        assert!(!job_id.is_empty());

        // 2. Query status for the newly queued job
        let req_status = Request::builder()
            .method("GET")
            .uri(&format!("/build/{}/status", job_id))
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::empty())
            .unwrap();

        let res_status = app.clone().oneshot(req_status).await.unwrap();
        assert_eq!(res_status.status(), StatusCode::OK);

        let status_bytes = axum::body::to_bytes(res_status.into_body(), usize::MAX).await.unwrap();
        let status_json: Value = serde_json::from_slice(&status_bytes).unwrap();
        assert_eq!(status_json["status"], "queued");
        assert_eq!(status_json["position"], 1); // first in line

        // 3. Fail on wrong/invalid job ID status query
        let req_not_found = Request::builder()
            .method("GET")
            .uri("/build/non-existent-id/status")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::empty())
            .unwrap();

        let res_not_found = app.oneshot(req_not_found).await.unwrap();
        assert_eq!(res_not_found.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_e2e_authorized_job_handling_without_binary_field() {
        let (app, _conn, _, _rx) = build_test_router();

        // Submit request without "binary" field (defaults to None)
        let payload = r#"{
            "project": "https://github.com/example/lib",
            "git_ref": "v1.0",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
        let job_id = body_json["id"].as_str().unwrap();

        let req_status = Request::builder()
            .method("GET")
            .uri(&format!("/build/{}/status", job_id))
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::empty())
            .unwrap();

        let res_status = app.oneshot(req_status).await.unwrap();
        assert_eq!(res_status.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_e2e_authorized_job_handling_with_binary_field() {
        let (app, _conn, _, _rx) = build_test_router();

        // Submit request with "binary" field
        let payload = r#"{
            "project": "https://github.com/example/lib",
            "git_ref": "v1.0",
            "binary": "mybinary",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
        let job_id = body_json["id"].as_str().unwrap();

        let req_status = Request::builder()
            .method("GET")
            .uri(&format!("/build/{}/status", job_id))
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::empty())
            .unwrap();

        let res_status = app.oneshot(req_status).await.unwrap();
        assert_eq!(res_status.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_integration_case_21_invalid_binary_name() {
        let (app, _conn, _queue, _rx) = build_test_router();

        let payload = r#"{
            "project": "https://github.com/example/lib",
            "git_ref": "v1.0",
            "binary": "-invalid",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_integration_case_22_valid_binary_name() {
        let (app, _conn, _queue, _rx) = build_test_router();

        let payload = r#"{
            "project": "https://github.com/example/lib",
            "git_ref": "v1.0",
            "binary": "valid-name-123",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn test_e2e_build_cache_behaviors() {
        let (app, conn_mutex, _queue, _rx) = build_test_router();

        // 1. Submit initial request
        let payload = r#"{
            "project": "https://github.com/example/cachetest",
            "git_ref": "v1.1",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req1 = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload))
            .unwrap();

        let res1 = app.clone().oneshot(req1).await.unwrap();
        assert_eq!(res1.status(), StatusCode::ACCEPTED);

        let body_bytes = axum::body::to_bytes(res1.into_body(), usize::MAX).await.unwrap();
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
        let job_id1 = body_json["id"].as_str().unwrap().to_string();

        // Create the temp artifacts directory
        let temp_dir = std::env::temp_dir().join("artifacts");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let dummy_archive = temp_dir.join(format!("{}.tar.gz", job_id1));
        std::fs::write(&dummy_archive, b"dummy compiled binaries content").unwrap();

        // Register done state and artifact in DB
        {
            let conn = conn_mutex.lock().unwrap();
            db::update_job_status(&conn, &job_id1, "done", None, Some("2026-05-17T17:00:00Z"), None).unwrap();
            db::insert_artifact(&conn, &job_id1, &dummy_archive.to_string_lossy(), 100, "dummysha").unwrap();

            // Compute cache key and insert cache entry
            let hardware = schema::HardwareProfile {
                cpu: schema::CpuProfile {
                    flags: vec!["avx2".to_string()],
                    cache_topology: "L1:32KB".to_string(),
                    core_count: 4,
                    ..Default::default()
                },
                memory: schema::MemoryProfile {
                    total_bytes: 8589934592,
                    available_bytes: 4294967296,
                    bandwidth_mbs: 12000.0,
                    ..Default::default()
                },
                storage: schema::StorageProfile {
                    io_uring: false,
                    o_direct: true,
                    read_speed_mbs: 450.0,
                    write_speed_mbs: 400.0,
                },
                gpu: schema::GpuProfile { devices: vec![] },
                ..Default::default()
            };
            let hw_str = serde_json::to_string(&hardware).unwrap();
            let cache_key = crate::cache::compute_cache_key(&hw_str, "https://github.com/example/cachetest", "v1.1", None, None, None);
            db::insert_cache_entry(&conn, &cache_key, &job_id1, "2026-05-17T17:00:00Z").unwrap();
        }

        // 2. Submit identical request -> must hit build cache!
        let req2 = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload))
            .unwrap();

        let res2 = app.clone().oneshot(req2).await.unwrap();
        assert_eq!(res2.status(), StatusCode::ACCEPTED);

        let body_bytes2 = axum::body::to_bytes(res2.into_body(), usize::MAX).await.unwrap();
        let body_json2: Value = serde_json::from_slice(&body_bytes2).unwrap();
        let job_id2 = body_json2["id"].as_str().unwrap().to_string();

        assert_eq!(job_id1, job_id2); // CACHE HIT!

        // 3. Remove physical archive -> must miss cache due to missing file fallback!
        std::fs::remove_file(&dummy_archive).unwrap();

        let req3 = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload))
            .unwrap();

        let res3 = app.clone().oneshot(req3).await.unwrap();
        assert_eq!(res3.status(), StatusCode::ACCEPTED);

        let body_bytes3 = axum::body::to_bytes(res3.into_body(), usize::MAX).await.unwrap();
        let body_json3: Value = serde_json::from_slice(&body_bytes3).unwrap();
        let job_id3 = body_json3["id"].as_str().unwrap().to_string();

        assert_ne!(job_id1, job_id3); // CACHE MISS (file deleted)!
    }

    #[tokio::test]
    async fn test_integration_package_routes() {
        let (app, conn_mutex, _queue, _rx) = build_test_router();

        // 22. POST /build without package field -> 202 (backward compatible)
        let payload_no_pkg = r#"{
            "project": "https://github.com/example/testproj",
            "git_ref": "main",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;
        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_no_pkg))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        // 23. POST /build with "package": "server" -> 202, job_id returned
        let payload_with_pkg = r#"{
            "project": "https://github.com/example/testproj",
            "git_ref": "main",
            "package": "server",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;
        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_with_pkg))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
        let job_id = body_json["id"].as_str().expect("ID must be string");
        assert!(!job_id.is_empty());

        // 24. POST /build with "binary": "probe" and "package": "cli" -> 202 (both accepted)
        let payload_both = r#"{
            "project": "https://github.com/example/testproj",
            "git_ref": "main",
            "binary": "probe",
            "package": "cli",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;
        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_both))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        // 25. POST /build with invalid token -> 401
        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer invalid_token")
            .body(Body::from(payload_with_pkg))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // 26. Two identical POST /build requests (same hardware, project, git_ref, binary, package) -> cache hit
        let payload_cache = r#"{
            "project": "https://github.com/example/cachepkg",
            "git_ref": "v2.0",
            "package": "core",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req1 = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_cache))
            .unwrap();
        let res1 = app.clone().oneshot(req1).await.unwrap();
        assert_eq!(res1.status(), StatusCode::ACCEPTED);
        let bytes1 = axum::body::to_bytes(res1.into_body(), usize::MAX).await.unwrap();
        let json1: Value = serde_json::from_slice(&bytes1).unwrap();
        let job_id1 = json1["id"].as_str().unwrap().to_string();

        // Create the temp archive to simulate build success
        let temp_dir = std::env::temp_dir().join("artifacts");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let dummy_archive = temp_dir.join(format!("{}.tar.gz", job_id1));
        std::fs::write(&dummy_archive, b"dummy compiled binaries content").unwrap();

        // Register done state and artifact in DB, and cache entry
        {
            let conn = conn_mutex.lock().unwrap();
            db::update_job_status(&conn, &job_id1, "done", None, Some("2026-05-17T17:00:00Z"), None).unwrap();
            db::insert_artifact(&conn, &job_id1, &dummy_archive.to_string_lossy(), 100, "dummysha").unwrap();

            let hardware = schema::HardwareProfile {
                cpu: schema::CpuProfile {
                    flags: vec!["avx2".to_string()],
                    cache_topology: "L1:32KB".to_string(),
                    core_count: 4,
                    ..Default::default()
                },
                memory: schema::MemoryProfile {
                    total_bytes: 8589934592,
                    available_bytes: 4294967296,
                    bandwidth_mbs: 12000.0,
                    ..Default::default()
                },
                storage: schema::StorageProfile {
                    io_uring: false,
                    o_direct: true,
                    read_speed_mbs: 450.0,
                    write_speed_mbs: 400.0,
                },
                gpu: schema::GpuProfile { devices: vec![] },
                ..Default::default()
            };
            let hw_str = serde_json::to_string(&hardware).unwrap();
            let cache_key = crate::cache::compute_cache_key(&hw_str, "https://github.com/example/cachepkg", "v2.0", None, Some("core"), None);
            db::insert_cache_entry(&conn, &cache_key, &job_id1, "2026-05-17T17:00:00Z").unwrap();
        }

        // Send second identical request -> cache hit
        let req2 = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_cache))
            .unwrap();
        let res2 = app.clone().oneshot(req2).await.unwrap();
        assert_eq!(res2.status(), StatusCode::ACCEPTED);
        let bytes2 = axum::body::to_bytes(res2.into_body(), usize::MAX).await.unwrap();
        let json2: Value = serde_json::from_slice(&bytes2).unwrap();
        let job_id2 = json2["id"].as_str().unwrap().to_string();

        assert_eq!(job_id1, job_id2); // CACHE HIT!
        std::fs::remove_file(&dummy_archive).unwrap();
    }

    #[tokio::test]
    async fn test_build_target_validation() {
        let (app, _conn, _queue, _rx) = build_test_router();

        // 14. target: None -> returns 202
        let payload_none = r#"{
            "project": "https://github.com/example/target_none",
            "git_ref": "v1.0",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;
        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_none))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        // 15. target: Some("aarch64-unknown-linux-gnu") -> returns 202
        let payload_valid = r#"{
            "project": "https://github.com/example/target_valid",
            "git_ref": "v1.0",
            "target": "aarch64-unknown-linux-gnu",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;
        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_valid))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        // 16. target: Some("unsupported-target") -> returns 400
        let payload_invalid = r#"{
            "project": "https://github.com/example/target_invalid",
            "git_ref": "v1.0",
            "target": "unsupported-target",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;
        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_invalid))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // 17. target: Some("") -> returns 400
        let payload_empty = r#"{
            "project": "https://github.com/example/target_empty",
            "git_ref": "v1.0",
            "target": "",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;
        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_empty))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_install_script() {
        let (app, _, _, _rx) = build_test_router();

        let req = Request::builder()
            .method("GET")
            .uri("/install/myapp?ref=v1.0.0&token=test_bearer")
            .body(Body::empty())
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        
        let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(body_str.contains("PROJECT=\"myapp\""));
        assert!(body_str.contains("REF=\"v1.0.0\""));
        assert!(body_str.contains("TOKEN=\"test_bearer\""));
    }

    #[tokio::test]
    async fn test_get_static_probe() {
        let (app, _, _, _rx) = build_test_router();

        let req = Request::builder()
            .method("GET")
            .uri("/probe/static/x86_64")
            .body(Body::empty())
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_post_forge_install() {
        let (app, _, _, _rx) = build_test_router();

        let payload = r#"{
            "cpu": {
                "flags": ["avx2"],
                "cache_topology": "L1d:32K",
                "core_count": 4,
                "cache_line_size": 64,
                "kernel_version": "Linux",
                "cpu_base_freq_mhz": 2000,
                "cpu_max_freq_mhz": 3000
            },
            "memory": {
                "total_bytes": 8589934592,
                "available_bytes": 4294967296,
                "bandwidth_mbs": 12000.0,
                "latency_ns_l1": 1.5,
                "latency_ns_l2": 3.5,
                "latency_ns_l3": 15.0,
                "latency_ns_ram": 75.0
            },
            "storage": {
                "io_uring": false,
                "o_direct": true,
                "read_speed_mbs": 450.0,
                "write_speed_mbs": 400.0
            },
            "gpu": {
                "devices": []
            }
        }"#;

        // Verify unauthorized requests fail!
        let req_unauth = Request::builder()
            .method("POST")
            .uri("/forge/install?project=myapp&ref=v1.0.0")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload))
            .unwrap();

        let res_unauth = app.clone().oneshot(req_unauth).await.unwrap();
        assert_eq!(res_unauth.status(), StatusCode::UNAUTHORIZED);

        // Verify authorized request works
        let req = Request::builder()
            .method("POST")
            .uri("/forge/install?project=myapp&ref=v1.0.0")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body_json.get("status").is_some());
    }

    #[tokio::test]
    async fn test_integration_pgo_build_phases() {
        let (app, conn_mutex, _queue, _rx) = build_test_router();

        // 11. POST /build with pgo_phase: Some("instrument") -> 202 Accepted, job_type pgo_instrument in DB
        let payload_instrument = r#"{
            "project": "https://github.com/example/pgotest",
            "git_ref": "v1.0",
            "pgo_phase": "instrument",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_instrument))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
        let job_id = body_json["id"].as_str().expect("ID must be string");
        
        {
            let conn = conn_mutex.lock().unwrap();
            let job_status = db::get_job_status(&conn, job_id).unwrap().expect("Job should exist");
            assert_eq!(job_status.job_type, "pgo_instrument");
        }

        // 12. POST /build with pgo_phase: Some("optimize") -> 400 Bad Request
        let payload_optimize = r#"{
            "project": "https://github.com/example/pgotest",
            "git_ref": "v1.0",
            "pgo_phase": "optimize",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_optimize))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // 13. POST /build with pgo_phase: Some("invalid") -> 400 Bad Request
        let payload_invalid = r#"{
            "project": "https://github.com/example/pgotest",
            "git_ref": "v1.0",
            "pgo_phase": "invalid",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_invalid))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // 14. POST /build with pgo_phase: Some("instrument") does NOT trigger build cache hit even when standard identical build exists
        // Setup standard build first in cache
        let payload_std = r#"{
            "project": "https://github.com/example/pgotest",
            "git_ref": "v1.0",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req_std = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_std))
            .unwrap();

        let res_std = app.clone().oneshot(req_std).await.unwrap();
        assert_eq!(res_std.status(), StatusCode::ACCEPTED);
        let bytes_std = axum::body::to_bytes(res_std.into_body(), usize::MAX).await.unwrap();
        let json_std: Value = serde_json::from_slice(&bytes_std).unwrap();
        let std_job_id = json_std["id"].as_str().unwrap().to_string();

        // Create standard artifact and populate cache
        let temp_dir = std::env::temp_dir().join("artifacts");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let dummy_archive = temp_dir.join(format!("{}.tar.gz", std_job_id));
        std::fs::write(&dummy_archive, b"compiled standard binary").unwrap();

        {
            let conn = conn_mutex.lock().unwrap();
            db::update_job_status(&conn, &std_job_id, "done", None, Some("2026-05-17T17:00:00Z"), None).unwrap();
            db::insert_artifact(&conn, &std_job_id, &dummy_archive.to_string_lossy(), 100, "dummysha").unwrap();

            let hardware = schema::HardwareProfile {
                cpu: schema::CpuProfile {
                    flags: vec!["avx2".to_string()],
                    cache_topology: "L1:32KB".to_string(),
                    core_count: 4,
                    ..Default::default()
                },
                memory: schema::MemoryProfile {
                    total_bytes: 8589934592,
                    available_bytes: 4294967296,
                    bandwidth_mbs: 12000.0,
                    ..Default::default()
                },
                storage: schema::StorageProfile {
                    io_uring: false,
                    o_direct: true,
                    read_speed_mbs: 450.0,
                    write_speed_mbs: 400.0,
                },
                gpu: schema::GpuProfile { devices: vec![] },
                ..Default::default()
            };
            let hw_str = serde_json::to_string(&hardware).unwrap();
            let cache_key = crate::cache::compute_cache_key(&hw_str, "https://github.com/example/pgotest", "v1.0", None, None, None);
            db::insert_cache_entry(&conn, &cache_key, &std_job_id, "2026-05-17T17:00:00Z").unwrap();
        }

        // Request instrumented build - must NOT hit cache
        let req_inst = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload_instrument))
            .unwrap();

        let res_inst = app.clone().oneshot(req_inst).await.unwrap();
        assert_eq!(res_inst.status(), StatusCode::ACCEPTED);
        let bytes_inst = axum::body::to_bytes(res_inst.into_body(), usize::MAX).await.unwrap();
        let json_inst: Value = serde_json::from_slice(&bytes_inst).unwrap();
        let inst_job_id = json_inst["id"].as_str().unwrap().to_string();

        assert_ne!(std_job_id, inst_job_id); // CACHE BYPASSED!
        std::fs::remove_file(&dummy_archive).unwrap();
    }

    #[tokio::test]
    async fn test_integration_pgo_profiles_errors() {
        let (app, conn_mutex, _queue, _rx) = build_test_router();

        // 18. POST /pgo/profiles/non-existent -> 404
        let boundary = "------------------------1234567890";
        let body_content = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"profile1\"; filename=\"test.profraw\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n\
             dummy content\r\n\
             --{boundary}--\r\n"
        );
        let req = Request::builder()
            .method("POST")
            .uri("/pgo/profiles/non-existent-id")
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(body_content.clone()))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);

        // Create job but with standard type instead of pgo_instrument
        let job_std_id = "job-std-123";
        let hardware = schema::HardwareProfile { ..Default::default() };
        {
            let conn = conn_mutex.lock().unwrap();
            db::insert_job(
                &conn,
                job_std_id,
                1,
                "https://github.com/example/pgotest",
                "v1.0",
                &hardware,
                "2026-05-17T17:00:00Z",
                "standard",
                None,
            ).unwrap();
            db::update_job_status(&conn, job_std_id, "done", None, None, None).unwrap();
        }

        // Rejects because not pgo_instrument -> 400
        let req = Request::builder()
            .method("POST")
            .uri(&format!("/pgo/profiles/{}", job_std_id))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(body_content.clone()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // Create job with pgo_instrument type but status "queued"
        let job_queued_id = "job-queued-123";
        {
            let conn = conn_mutex.lock().unwrap();
            db::insert_job(
                &conn,
                job_queued_id,
                1,
                "https://github.com/example/pgotest",
                "v1.0",
                &hardware,
                "2026-05-17T17:00:00Z",
                "pgo_instrument",
                None,
            ).unwrap();
        }

        // 17. Rejects because not done -> 400 Bad Request
        let req = Request::builder()
            .method("POST")
            .uri(&format!("/pgo/profiles/{}", job_queued_id))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(body_content.clone()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // Update queued job to "done"
        {
            let conn = conn_mutex.lock().unwrap();
            db::update_job_status(&conn, job_queued_id, "done", None, None, None).unwrap();
        }

        // 16. Rejects non-.profraw files -> 400 Bad Request
        let body_invalid_file = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"profile1\"; filename=\"test.txt\"\r\n\
             Content-Type: text/plain\r\n\r\n\
             dummy content\r\n\
             --{boundary}--\r\n"
        );
        let req = Request::builder()
            .method("POST")
            .uri(&format!("/pgo/profiles/{}", job_queued_id))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(body_invalid_file))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_integration_case_23_invalid_project_url() {
        let (app, conn_mutex, _queue, mut rx) = build_test_router();

        let payload = r#"{
            "project": "file:///etc/passwd",
            "git_ref": "main",
            "hardware": {
                "cpu": {"flags": ["avx2"], "cache_topology": "L1:32KB", "core_count": 4},
                "memory": {"total_bytes": 8589934592, "available_bytes": 4294967296, "bandwidth_mbs": 12000.0},
                "storage": {"io_uring": false, "o_direct": true, "read_speed_mbs": 450.0, "write_speed_mbs": 400.0},
                "gpu": {"devices": []}
            }
        }"#;

        let req = Request::builder()
            .method("POST")
            .uri("/build")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(payload))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
        let job_id = body_json["id"].as_str().unwrap();

        let job = rx.recv().await.expect("Job should be queued");
        assert_eq!(job.id, job_id);

        let artifacts_dir = std::env::temp_dir();
        let res_process = crate::worker::process_job(&conn_mutex, job, &artifacts_dir);
        assert!(res_process.is_ok());

        let conn = conn_mutex.lock().unwrap();
        let job_status = db::get_job_status(&conn, job_id).unwrap().unwrap();
        assert_eq!(job_status.status, "failed");
        assert_eq!(job_status.error_msg.unwrap(), "Only https:// project URLs are permitted");
    }

    #[tokio::test]
    async fn test_secure_default_token_generation_case_25() {
        let conn = db::init_db(":memory:").unwrap();

        std::env::set_var("KOVAL_ADMIN_TOKEN", "test-env-token-123");
        let token_from_env = bootstrap_admin_token(&conn).unwrap().unwrap();
        assert_eq!(token_from_env, "test-env-token-123");

        let active_tokens = db::get_active_tokens(&conn).unwrap();
        assert_eq!(active_tokens.len(), 1);
        assert_eq!(active_tokens[0].name, "Default Admin Token");

        let authenticated_token = auth::authenticate_and_rate_limit(
            &conn,
            "test-env-token-123",
            chrono::Utc::now(),
            100,
        ).unwrap();
        assert_eq!(authenticated_token.name, "Default Admin Token");

        std::env::remove_var("KOVAL_ADMIN_TOKEN");
        let conn2 = db::init_db(":memory:").unwrap();

        let generated_token = bootstrap_admin_token(&conn2).unwrap().unwrap();
        assert!(uuid::Uuid::parse_str(&generated_token).is_ok());

        let authenticated_token2 = auth::authenticate_and_rate_limit(
            &conn2,
            &generated_token,
            chrono::Utc::now(),
            100,
        ).unwrap();
        assert_eq!(authenticated_token2.name, "Default Admin Token");
    }

    #[tokio::test]
    async fn test_integration_pgo_upload_limits_case_31() {
        let (app, conn_mutex, _queue, _rx) = build_test_router();

        let job_id = "job-limit-123";
        let hardware = schema::HardwareProfile { ..Default::default() };
        {
            let conn = conn_mutex.lock().unwrap();
            db::insert_job(
                &conn,
                job_id,
                1,
                "https://github.com/example/pgotest",
                "v1.0",
                &hardware,
                "2026-05-17T17:00:00Z",
                "pgo_instrument",
                None,
            ).unwrap();
            db::update_job_status(&conn, job_id, "done", None, None, None).unwrap();
        }

        let boundary = "------------------------1234567890";
        let mut multipart_body = Vec::new();
        for i in 1..=33 {
            multipart_body.extend_from_slice(
                format!(
                    "--{boundary}\r\n\
                     Content-Disposition: form-data; name=\"profile{i}\"; filename=\"test{i}.profraw\"\r\n\
                     Content-Type: application/octet-stream\r\n\r\n\
                     dummy content\r\n"
                )
                .as_bytes(),
            );
        }
        multipart_body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

        let req = Request::builder()
            .method("POST")
            .uri(&format!("/pgo/profiles/{}", job_id))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(multipart_body))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        let large_body = vec![0u8; 51 * 1024 * 1024];
        let req_large = Request::builder()
            .method("POST")
            .uri(&format!("/pgo/profiles/{}", job_id))
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .header(header::AUTHORIZATION, "Bearer test_bearer")
            .body(Body::from(large_body))
            .unwrap();

        let res_large = app.oneshot(req_large).await.unwrap();
        let status_large = res_large.status();
        assert!(
            status_large == StatusCode::BAD_REQUEST || status_large == StatusCode::PAYLOAD_TOO_LARGE,
            "Expected BAD_REQUEST or PAYLOAD_TOO_LARGE, got {:?}",
            status_large
        );
    }
}
