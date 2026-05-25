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

use crate::queue::JobQueue;
use crate::routes::AppState;
use crate::worker::BuildWorker;

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
    let active_tokens = db::get_active_tokens(&conn).expect("Failed to check active tokens");
    if active_tokens.is_empty() {
        let default_admin_token = "koval_tkn_default_admin";
        let hashed = auth::hash_token(default_admin_token).expect("Failed to hash bootstrap token");
        db::insert_token(
            &conn,
            &hashed,
            "Default Admin Token",
            &chrono::Utc::now().to_rfc3339(),
        )
        .expect("Failed to bootstrap default admin token");
        
        println!("=======================================================");
        println!("  BOOTSTRAPPED DEFAULT DEVELOPER ADMIN TOKEN:");
        println!("  Bearer Token: {}", default_admin_token);
        println!("=======================================================");
    }

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
                },
                memory: schema::MemoryProfile {
                    total_bytes: 8589934592,
                    available_bytes: 4294967296,
                    bandwidth_mbs: 12000.0,
                },
                storage: schema::StorageProfile {
                    io_uring: false,
                    o_direct: true,
                    read_speed_mbs: 450.0,
                    write_speed_mbs: 400.0,
                },
                gpu: schema::GpuProfile { devices: vec![] },
            };
            let hw_str = serde_json::to_string(&hardware).unwrap();
            let cache_key = crate::cache::compute_cache_key(&hw_str, "https://github.com/example/cachetest", "v1.1", None, None);
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
                },
                memory: schema::MemoryProfile {
                    total_bytes: 8589934592,
                    available_bytes: 4294967296,
                    bandwidth_mbs: 12000.0,
                },
                storage: schema::StorageProfile {
                    io_uring: false,
                    o_direct: true,
                    read_speed_mbs: 450.0,
                    write_speed_mbs: 400.0,
                },
                gpu: schema::GpuProfile { devices: vec![] },
            };
            let hw_str = serde_json::to_string(&hardware).unwrap();
            let cache_key = crate::cache::compute_cache_key(&hw_str, "https://github.com/example/cachepkg", "v2.0", None, Some("core"));
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
}
