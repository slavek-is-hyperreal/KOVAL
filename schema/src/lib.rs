use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CpuProfile {
    pub flags: Vec<String>,
    pub cache_topology: String,
    pub core_count: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct MemoryProfile {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub bandwidth_mbs: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StorageProfile {
    pub io_uring: bool,
    pub o_direct: bool,
    pub read_speed_mbs: f64,
    pub write_speed_mbs: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct VulkanDeviceProfile {
    pub name: String,
    pub vram_bytes: u64,
    pub pcie_info: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct GpuProfile {
    pub devices: Vec<VulkanDeviceProfile>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct HardwareProfile {
    pub cpu: CpuProfile,
    pub memory: MemoryProfile,
    pub storage: StorageProfile,
    pub gpu: GpuProfile,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct JobRequest {
    pub hardware: HardwareProfile,
    pub project: String,
    pub git_ref: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct JobStatus {
    pub status: String, // "queued" | "building" | "done" | "failed"
    pub queued_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error_msg: Option<String>,
    pub position: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Token {
    pub id: i64,
    pub token_hash: String,
    pub name: String,
    pub created_at: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WebhookRequest {
    pub url: String,
    pub secret: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WebhookRecord {
    pub id: i64,
    pub url: String,
    pub created_at: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WebhookPayload {
    pub job_id: String,
    pub status: String,
    pub finished_at: Option<String>,
    pub project: String,
    pub sha256: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct JobSummary {
    pub id: String,
    pub project: String,
    pub git_ref: String,
    pub status: String,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct TokenRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct TokenResponse {
    pub id: i64,
    pub plaintext_token: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct TokenRecord {
    pub id: i64,
    pub name: String,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hardware_profile_roundtrip() {
        let profile = HardwareProfile {
            cpu: CpuProfile {
                flags: vec!["avx2".to_string(), "sse4.1".to_string()],
                cache_topology: "L1:32KB, L2:256KB, L3:16MB".to_string(),
                core_count: 8,
            },
            memory: MemoryProfile {
                total_bytes: 17179869184,
                available_bytes: 8589934592,
                bandwidth_mbs: 25400.5,
            },
            storage: StorageProfile {
                io_uring: true,
                o_direct: true,
                read_speed_mbs: 3500.0,
                write_speed_mbs: 3000.0,
            },
            gpu: GpuProfile {
                devices: vec![VulkanDeviceProfile {
                    name: "NVIDIA GeForce RTX 4090".to_string(),
                    vram_bytes: 25769803776,
                    pcie_info: Some("PCIe 4.0 x16".to_string()),
                }],
            },
        };

        let serialized = serde_json::to_string(&profile).expect("Failed to serialize HardwareProfile");
        let deserialized: HardwareProfile = serde_json::from_str(&serialized).expect("Failed to deserialize HardwareProfile");

        assert_eq!(profile, deserialized);
    }

    #[test]
    fn test_job_request_roundtrip() {
        let profile = HardwareProfile {
            cpu: CpuProfile {
                flags: vec![],
                cache_topology: "".to_string(),
                core_count: 4,
            },
            memory: MemoryProfile {
                total_bytes: 8589934592,
                available_bytes: 4294967296,
                bandwidth_mbs: 12000.0,
            },
            storage: StorageProfile {
                io_uring: false,
                o_direct: false,
                read_speed_mbs: 500.0,
                write_speed_mbs: 450.0,
            },
            gpu: GpuProfile { devices: vec![] },
        };

        let request = JobRequest {
            hardware: profile,
            project: "https://github.com/example/project.git".to_string(),
            git_ref: "main".to_string(),
        };

        let serialized = serde_json::to_string(&request).expect("Failed to serialize JobRequest");
        let deserialized: JobRequest = serde_json::from_str(&serialized).expect("Failed to deserialize JobRequest");

        assert_eq!(request, deserialized);
    }

    #[test]
    fn test_job_status_roundtrip() {
        let status = JobStatus {
            status: "queued".to_string(),
            queued_at: "2026-05-17T16:53:00Z".to_string(),
            started_at: None,
            finished_at: None,
            error_msg: None,
            position: Some(2),
        };

        let serialized = serde_json::to_string(&status).expect("Failed to serialize JobStatus");
        let deserialized: JobStatus = serde_json::from_str(&serialized).expect("Failed to deserialize JobStatus");

        assert_eq!(status, deserialized);
    }

    #[test]
    fn test_token_roundtrip() {
        let token = Token {
            id: 1,
            token_hash: "$2b$12$somehash".to_string(),
            name: "admin".to_string(),
            created_at: "2026-05-17T16:53:00Z".to_string(),
            is_active: true,
        };

        let serialized = serde_json::to_string(&token).expect("Failed to serialize Token");
        let deserialized: Token = serde_json::from_str(&serialized).expect("Failed to deserialize Token");

        assert_eq!(token, deserialized);
    }
}
