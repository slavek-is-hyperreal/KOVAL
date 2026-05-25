use rusqlite::{params, Connection, OptionalExtension};
use schema::{HardwareProfile, JobStatus, Token};
use std::path::Path;

pub fn init_db<P: AsRef<Path>>(path: P) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open(path)?;
    
    // Enable foreign keys
    conn.execute("PRAGMA foreign_keys = ON;", [])?;

    // Create tables
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tokens (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            token_hash TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            created_at TEXT NOT NULL,
            is_active INTEGER NOT NULL CHECK (is_active IN (0, 1))
        );",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY,
            token_id INTEGER NOT NULL,
            project TEXT NOT NULL,
            git_ref TEXT NOT NULL,
            hardware_json TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('queued', 'building', 'done', 'failed')),
            queued_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            error_msg TEXT,
            FOREIGN KEY(token_id) REFERENCES tokens(id)
        );",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS artifacts (
            job_id TEXT PRIMARY KEY,
            file_path TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            FOREIGN KEY(job_id) REFERENCES jobs(id) ON DELETE CASCADE
        );",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS rate_limit (
            token_id INTEGER NOT NULL,
            window_start TEXT NOT NULL,
            request_count INTEGER NOT NULL,
            PRIMARY KEY (token_id, window_start),
            FOREIGN KEY(token_id) REFERENCES tokens(id) ON DELETE CASCADE
        );",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS webhooks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            token_id INTEGER NOT NULL,
            url TEXT NOT NULL,
            secret TEXT NOT NULL,
            created_at TEXT NOT NULL,
            is_active INTEGER NOT NULL CHECK (is_active IN (0, 1)),
            FOREIGN KEY(token_id) REFERENCES tokens(id)
        );",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS build_cache (
            cache_key TEXT PRIMARY KEY,
            job_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY(job_id) REFERENCES jobs(id) ON DELETE CASCADE
        );",
        [],
    )?;

    Ok(conn)
}

// TOKEN QUERIES

pub fn insert_token(conn: &Connection, token_hash: &str, name: &str, created_at: &str) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO tokens (token_hash, name, created_at, is_active) VALUES (?1, ?2, ?3, 1)",
        params![token_hash, name, created_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_token_by_hash(conn: &Connection, token_hash: &str) -> Result<Option<Token>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, token_hash, name, created_at, is_active FROM tokens WHERE token_hash = ?1",
        params![token_hash],
        |row| {
            let active_int: i32 = row.get(4)?;
            Ok(Token {
                id: row.get(0)?,
                token_hash: row.get(1)?,
                name: row.get(2)?,
                created_at: row.get(3)?,
                is_active: active_int == 1,
            })
        },
    )
    .optional()
}

pub fn revoke_token(conn: &Connection, id: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE tokens SET is_active = 0 WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

// JOB QUERIES

pub fn insert_job(
    conn: &Connection,
    id: &str,
    token_id: i64,
    project: &str,
    git_ref: &str,
    hardware: &HardwareProfile,
    queued_at: &str,
) -> Result<(), rusqlite::Error> {
    let hardware_json = serde_json::to_string(hardware).unwrap_or_default();
    conn.execute(
        "INSERT INTO jobs (id, token_id, project, git_ref, hardware_json, status, queued_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6)",
        params![id, token_id, project, git_ref, hardware_json, queued_at],
    )?;
    Ok(())
}

pub fn get_job_status(conn: &Connection, id: &str) -> Result<Option<JobStatus>, rusqlite::Error> {
    let position = get_job_position(conn, id)?;
    
    conn.query_row(
        "SELECT status, queued_at, started_at, finished_at, error_msg FROM jobs WHERE id = ?1",
        params![id],
        |row| {
            Ok(JobStatus {
                status: row.get(0)?,
                queued_at: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3)?,
                error_msg: row.get(4)?,
                position,
            })
        },
    )
    .optional()
}

pub fn get_job_hardware(conn: &Connection, id: &str) -> Result<Option<HardwareProfile>, rusqlite::Error> {
    let json: Option<String> = conn
        .query_row(
            "SELECT hardware_json FROM jobs WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(json_str) = json {
        if let Ok(profile) = serde_json::from_str(&json_str) {
            return Ok(Some(profile));
        }
    }
    Ok(None)
}

pub fn get_job_position(conn: &Connection, id: &str) -> Result<Option<usize>, rusqlite::Error> {
    // Check if the job is indeed queued
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM jobs WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;

    match status {
        Some(s) if s == "queued" => {
            // Find how many jobs were queued BEFORE this job
            let queued_at: String = conn.query_row(
                "SELECT queued_at FROM jobs WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )?;
            
            let count: usize = conn.query_row(
                "SELECT COUNT(*) FROM jobs WHERE status = 'queued' AND queued_at < ?1",
                params![queued_at],
                |row| row.get(0),
            )?;
            
            Ok(Some(count + 1))
        }
        _ => Ok(None),
    }
}

pub fn update_job_status(
    conn: &Connection,
    id: &str,
    status: &str,
    started_at: Option<&str>,
    finished_at: Option<&str>,
    error_msg: Option<&str>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE jobs SET status = ?2, started_at = COALESCE(?3, started_at), finished_at = ?4, error_msg = ?5 WHERE id = ?1",
        params![id, status, started_at, finished_at, error_msg],
    )?;
    Ok(())
}

// ARTIFACT QUERIES

pub fn insert_artifact(
    conn: &Connection,
    job_id: &str,
    file_path: &str,
    file_size: u64,
    sha256: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR REPLACE INTO artifacts (job_id, file_path, file_size, sha256) VALUES (?1, ?2, ?3, ?4)",
        params![job_id, file_path, file_size as i64, sha256],
    )?;
    Ok(())
}

pub fn get_artifact(conn: &Connection, job_id: &str) -> Result<Option<(String, u64, String)>, rusqlite::Error> {
    conn.query_row(
        "SELECT file_path, file_size, sha256 FROM artifacts WHERE job_id = ?1",
        params![job_id],
        |row| {
            let size_i64: i64 = row.get(1)?;
            Ok((row.get(0)?, size_i64 as u64, row.get(2)?))
        },
    )
    .optional()
}

// RATE LIMIT QUERIES

pub fn get_rate_limit(conn: &Connection, token_id: i64, window_start: &str) -> Result<usize, rusqlite::Error> {
    let count: Option<usize> = conn
        .query_row(
            "SELECT request_count FROM rate_limit WHERE token_id = ?1 AND window_start = ?2",
            params![token_id, window_start],
            |row| row.get(0),
        )
        .optional()?;
    Ok(count.unwrap_or(0))
}

pub fn increment_rate_limit(conn: &Connection, token_id: i64, window_start: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO rate_limit (token_id, window_start, request_count) VALUES (?1, ?2, 1)
         ON CONFLICT(token_id, window_start) DO UPDATE SET request_count = request_count + 1",
        params![token_id, window_start],
    )?;
    Ok(())
}

pub fn prune_rate_limits(conn: &Connection, before_window: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM rate_limit WHERE window_start < ?1",
        params![before_window],
    )?;
    Ok(())
}

pub fn get_sliding_window_count(conn: &Connection, token_id: i64, since_time: &str) -> Result<usize, rusqlite::Error> {
    let count: usize = conn.query_row(
        "SELECT COALESCE(SUM(request_count), 0) FROM rate_limit WHERE token_id = ?1 AND window_start >= ?2",
        params![token_id, since_time],
        |row| row.get(0),
    )?;
    Ok(count)
}

pub fn get_active_tokens(conn: &Connection) -> Result<Vec<Token>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT id, token_hash, name, created_at, is_active FROM tokens WHERE is_active = 1")?;
    let token_iter = stmt.query_map([], |row| {
        Ok(Token {
            id: row.get(0)?,
            token_hash: row.get(1)?,
            name: row.get(2)?,
            created_at: row.get(3)?,
            is_active: true,
        })
    })?;
    
    let mut tokens = Vec::new();
    for token in token_iter {
        tokens.push(token?);
    }
    Ok(tokens)
}

// WEBHOOK QUERIES

pub fn insert_webhook(
    conn: &Connection,
    token_id: i64,
    url: &str,
    secret: &str,
    created_at: &str,
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO webhooks (token_id, url, secret, created_at, is_active) VALUES (?1, ?2, ?3, ?4, 1)",
        params![token_id, url, secret, created_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_webhooks_for_token(
    conn: &Connection,
    token_id: i64,
) -> Result<Vec<schema::WebhookRecord>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, url, created_at, is_active FROM webhooks WHERE token_id = ?1 AND is_active = 1"
    )?;
    let iter = stmt.query_map(params![token_id], |row| {
        let is_active_int: i32 = row.get(3)?;
        Ok(schema::WebhookRecord {
            id: row.get(0)?,
            url: row.get(1)?,
            created_at: row.get(2)?,
            is_active: is_active_int == 1,
        })
    })?;
    let mut list = Vec::new();
    for item in iter {
        list.push(item?);
    }
    Ok(list)
}

pub fn deactivate_webhook(
    conn: &Connection,
    id: i64,
    token_id: i64,
) -> Result<(), rusqlite::Error> {
    let rows_affected = conn.execute(
        "UPDATE webhooks SET is_active = 0 WHERE id = ?1 AND token_id = ?2",
        params![id, token_id],
    )?;
    if rows_affected == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

pub fn get_webhooks_delivery_info(
    conn: &Connection,
    token_id: i64,
) -> Result<Vec<(String, String)>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT url, secret FROM webhooks WHERE token_id = ?1 AND is_active = 1"
    )?;
    let iter = stmt.query_map(params![token_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut list = Vec::new();
    for item in iter {
        list.push(item?);
    }
    Ok(list)
}

// ADDITIONAL TOKEN ADMIN QUERIES

pub fn get_active_token_records(
    conn: &Connection,
) -> Result<Vec<schema::TokenRecord>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, name, created_at FROM tokens WHERE is_active = 1"
    )?;
    let iter = stmt.query_map([], |row| {
        Ok(schema::TokenRecord {
            id: row.get(0)?,
            name: row.get(1)?,
            created_at: row.get(2)?,
        })
    })?;
    let mut list = Vec::new();
    for item in iter {
        list.push(item?);
    }
    Ok(list)
}

// RECENT JOBS QUERY

pub fn get_recent_jobs(
    conn: &Connection,
    token_id: i64,
    limit: usize,
) -> Result<Vec<schema::JobSummary>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, project, git_ref, status, queued_at, started_at, finished_at 
         FROM jobs 
         WHERE token_id = ?1 
         ORDER BY queued_at DESC 
         LIMIT ?2"
    )?;
    let iter = stmt.query_map(params![token_id, limit as i64], |row| {
        Ok(schema::JobSummary {
            id: row.get(0)?,
            project: row.get(1)?,
            git_ref: row.get(2)?,
            status: row.get(3)?,
            queued_at: row.get(4)?,
            started_at: row.get(5)?,
            finished_at: row.get(6)?,
        })
    })?;
    let mut list = Vec::new();
    for item in iter {
        list.push(item?);
    }
    Ok(list)
}

// BUILD_CACHE QUERIES

pub fn get_cache_entry(conn: &Connection, cache_key: &str) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT job_id FROM build_cache WHERE cache_key = ?1",
        params![cache_key],
        |row| row.get(0),
    )
    .optional()
}

pub fn insert_cache_entry(
    conn: &Connection,
    cache_key: &str,
    job_id: &str,
    created_at: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR REPLACE INTO build_cache (cache_key, job_id, created_at) VALUES (?1, ?2, ?3)",
        params![cache_key, job_id, created_at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema::{CpuProfile, GpuProfile, MemoryProfile, StorageProfile};

    fn setup_mem_db() -> Connection {
        let conn = init_db(":memory:").expect("Failed to create in-memory database");
        conn
    }

    #[test]
    fn test_db_token_management() {
        let conn = setup_mem_db();
        let token_hash = "$2b$12$somehashforadmin".to_string();
        let created_at = "2026-05-17T16:53:00Z";

        // Insert token
        let token_id = insert_token(&conn, &token_hash, "Admin Token", created_at).unwrap();
        assert!(token_id > 0);

        // Fetch token and verify
        let token = get_token_by_hash(&conn, &token_hash).unwrap().expect("Token should exist");
        assert_eq!(token.id, token_id);
        assert_eq!(token.name, "Admin Token");
        assert!(token.is_active);

        // Revoke token
        revoke_token(&conn, token_id).unwrap();

        // Verify token is inactive
        let token_after = get_token_by_hash(&conn, &token_hash).unwrap().expect("Token should exist");
        assert!(!token_after.is_active);
    }

    #[test]
    fn test_db_job_state_transitions() {
        let conn = setup_mem_db();
        let token_hash = "$2b$12$somehash".to_string();
        let token_id = insert_token(&conn, &token_hash, "Developer", "2026-05-17T16:53:00Z").unwrap();

        let hardware = HardwareProfile {
            cpu: CpuProfile {
                flags: vec!["avx2".to_string()],
                cache_topology: "".to_string(),
                core_count: 4,
                ..Default::default()
            },
            memory: MemoryProfile {
                total_bytes: 8192,
                available_bytes: 4096,
                bandwidth_mbs: 1000.0,
            },
            storage: StorageProfile {
                io_uring: false,
                o_direct: false,
                read_speed_mbs: 100.0,
                write_speed_mbs: 100.0,
            },
            gpu: GpuProfile { devices: vec![] },
            ..Default::default()
        };

        let job_id = "job-uuid-1234";

        // Insert Job
        insert_job(&conn, job_id, token_id, "https://github.com/test/repo", "main", &hardware, "2026-05-17T16:53:00Z").unwrap();

        // Verify status and position
        let status = get_job_status(&conn, job_id).unwrap().expect("Job should exist");
        assert_eq!(status.status, "queued");
        assert_eq!(status.position, Some(1));
        assert!(status.started_at.is_none());

        // Update to building
        update_job_status(&conn, job_id, "building", Some("2026-05-17T16:54:00Z"), None, None).unwrap();
        let status_building = get_job_status(&conn, job_id).unwrap().expect("Job should exist");
        assert_eq!(status_building.status, "building");
        assert_eq!(status_building.position, None); // building jobs have no queue position
        assert_eq!(status_building.started_at, Some("2026-05-17T16:54:00Z".to_string()));

        // Update to done
        update_job_status(&conn, job_id, "done", None, Some("2026-05-17T16:55:00Z"), None).unwrap();
        let status_done = get_job_status(&conn, job_id).unwrap().expect("Job should exist");
        assert_eq!(status_done.status, "done");
        assert_eq!(status_done.finished_at, Some("2026-05-17T16:55:00Z".to_string()));
    }

    #[test]
    fn test_db_webhooks_and_recent_jobs() {
        let conn = setup_mem_db();
        let token_hash = "$2b$12$webhookstoken".to_string();
        let token_id = insert_token(&conn, &token_hash, "Webhook Tester", "2026-05-17T16:53:00Z").unwrap();

        // 1. Insert and list webhooks
        let wh_id = insert_webhook(&conn, token_id, "https://example.com/webhook", "super_secret", "2026-05-17T16:53:00Z").unwrap();
        assert!(wh_id > 0);

        let list = get_webhooks_for_token(&conn, token_id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, wh_id);
        assert_eq!(list[0].url, "https://example.com/webhook");
        assert!(list[0].is_active);

        // Check delivery info (url, secret)
        let delivery = get_webhooks_delivery_info(&conn, token_id).unwrap();
        assert_eq!(delivery.len(), 1);
        assert_eq!(delivery[0].0, "https://example.com/webhook");
        assert_eq!(delivery[0].1, "super_secret");

        // 2. Deactivate webhook
        deactivate_webhook(&conn, wh_id, token_id).unwrap();
        let list_after = get_webhooks_for_token(&conn, token_id).unwrap();
        assert_eq!(list_after.len(), 0);

        // 3. Test recent jobs list
        let hardware = HardwareProfile {
            cpu: CpuProfile { flags: vec![], cache_topology: "".to_string(), core_count: 1, ..Default::default() },
            memory: MemoryProfile { total_bytes: 1024, available_bytes: 512, bandwidth_mbs: 100.0 },
            storage: StorageProfile { io_uring: false, o_direct: false, read_speed_mbs: 10.0, write_speed_mbs: 10.0 },
            gpu: GpuProfile { devices: vec![] },
            ..Default::default()
        };
        insert_job(&conn, "job-1", token_id, "project1", "ref1", &hardware, "2026-05-17T16:53:00Z").unwrap();
        insert_job(&conn, "job-2", token_id, "project2", "ref2", &hardware, "2026-05-17T16:54:00Z").unwrap();

        let recent = get_recent_jobs(&conn, token_id, 5).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, "job-2"); // ordered descending by queued_at
        assert_eq!(recent[1].id, "job-1");
    }

    #[test]
    fn test_db_build_cache() {
        let conn = setup_mem_db();
        let token_hash = "$2b$12$cachetoken".to_string();
        let token_id = insert_token(&conn, &token_hash, "Cache Client", "2026-05-17T16:53:00Z").unwrap();

        let hardware = HardwareProfile {
            cpu: CpuProfile { flags: vec![], cache_topology: "".to_string(), core_count: 1, ..Default::default() },
            memory: MemoryProfile { total_bytes: 1024, available_bytes: 512, bandwidth_mbs: 100.0 },
            storage: StorageProfile { io_uring: false, o_direct: false, read_speed_mbs: 10.0, write_speed_mbs: 10.0 },
            gpu: GpuProfile { devices: vec![] },
            ..Default::default()
        };
        let job_id = "job-uuid-cache";
        insert_job(&conn, job_id, token_id, "project", "ref", &hardware, "2026-05-17T16:53:00Z").unwrap();

        let cache_key = "my-awesome-cache-key-123";
        let created_at = "2026-05-17T16:53:00Z";

        let missing = get_cache_entry(&conn, cache_key).unwrap();
        assert!(missing.is_none());

        insert_cache_entry(&conn, cache_key, job_id, created_at).unwrap();

        let found = get_cache_entry(&conn, cache_key).unwrap().expect("Cache entry should exist");
        assert_eq!(found, job_id);
    }
}
