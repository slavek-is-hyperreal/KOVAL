use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use rusqlite::Connection;
use crate::queue::JobQueue;

pub mod build;
pub mod status;
pub mod binary;

#[derive(Clone)]
pub struct AppState {
    pub conn: Arc<Mutex<Connection>>,
    pub queue: Arc<JobQueue>,
    pub artifacts_dir: PathBuf,
    pub rate_limit_limit: usize,
}
