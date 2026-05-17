use tokio::sync::mpsc::{channel, Receiver, Sender, error::TrySendError};
use schema::HardwareProfile;

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub token_id: i64,
    pub project: String,
    pub git_ref: String,
    pub hardware: HardwareProfile,
    pub binary: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum QueueError {
    QueueFull,
    SendError(String),
}

pub struct JobQueue {
    sender: Sender<Job>,
    capacity: usize,
}

impl JobQueue {
    /// Creates a new job queue with the given capacity
    pub fn new(capacity: usize) -> (Self, Receiver<Job>) {
        let (tx, rx) = channel(capacity);
        (
            Self {
                sender: tx,
                capacity,
            },
            rx,
        )
    }

    /// Enqueues a job, returning the job ID if successful, or QueueFull if the queue is at capacity
    pub fn enqueue(&self, job: Job) -> Result<String, QueueError> {
        let id = job.id.clone();
        
        // try_send returns immediately without waiting, enforcing strict backpressure
        match self.sender.try_send(job) {
            Ok(()) => Ok(id),
            Err(TrySendError::Full(_)) => Err(QueueError::QueueFull),
            Err(TrySendError::Closed(_)) => Err(QueueError::SendError("Dispatcher queue closed".to_string())),
        }
    }

    /// Gets the maximum capacity of the queue
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema::{CpuProfile, GpuProfile, MemoryProfile, StorageProfile};

    fn get_dummy_hardware() -> HardwareProfile {
        HardwareProfile {
            cpu: CpuProfile {
                flags: vec![],
                cache_topology: "".to_string(),
                core_count: 2,
            },
            memory: MemoryProfile {
                total_bytes: 4096,
                available_bytes: 2048,
                bandwidth_mbs: 100.0,
            },
            storage: StorageProfile {
                io_uring: false,
                o_direct: false,
                read_speed_mbs: 50.0,
                write_speed_mbs: 50.0,
            },
            gpu: GpuProfile { devices: vec![] },
        }
    }

    #[tokio::test]
    async fn test_queue_operations_and_backpressure() {
        // Create a queue with capacity 2
        let (queue, mut receiver) = JobQueue::new(2);

        let job1 = Job {
            id: "job-1".to_string(),
            token_id: 1,
            project: "repo1".to_string(),
            git_ref: "master".to_string(),
            hardware: get_dummy_hardware(),
            binary: None,
        };

        let job2 = Job {
            id: "job-2".to_string(),
            token_id: 1,
            project: "repo2".to_string(),
            git_ref: "master".to_string(),
            hardware: get_dummy_hardware(),
            binary: None,
        };

        let job3 = Job {
            id: "job-3".to_string(),
            token_id: 1,
            project: "repo3".to_string(),
            git_ref: "master".to_string(),
            hardware: get_dummy_hardware(),
            binary: None,
        };

        // 1. Queueing returns correct ID
        assert_eq!(queue.enqueue(job1).unwrap(), "job-1");
        assert_eq!(queue.enqueue(job2).unwrap(), "job-2");

        // 2. Full queue rejects immediately (backpressure)
        let err = queue.enqueue(job3);
        assert_eq!(err, Err(QueueError::QueueFull));

        // 3. Worker pulls job successfully from queue
        let pulled1 = receiver.recv().await.expect("Should receive job 1");
        assert_eq!(pulled1.id, "job-1");

        // Since we pulled 1 job, we can now enqueue another
        let job3_retry = Job {
            id: "job-3".to_string(),
            token_id: 1,
            project: "repo3".to_string(),
            git_ref: "master".to_string(),
            hardware: get_dummy_hardware(),
            binary: None,
        };
        assert_eq!(queue.enqueue(job3_retry).unwrap(), "job-3");

        let pulled2 = receiver.recv().await.expect("Should receive job 2");
        assert_eq!(pulled2.id, "job-2");

        let pulled3 = receiver.recv().await.expect("Should receive job 3");
        assert_eq!(pulled3.id, "job-3");
    }
}
