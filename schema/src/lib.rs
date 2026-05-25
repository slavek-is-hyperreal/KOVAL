use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct CpuProfile {
    pub flags: Vec<String>,
    pub cache_topology: String,
    pub core_count: usize,
    #[serde(default)]
    pub cache_line_size: Option<u64>,
    #[serde(default)]
    pub kernel_version: Option<String>,
    #[serde(default)]
    pub cpu_base_freq_mhz: Option<u64>,
    #[serde(default)]
    pub cpu_max_freq_mhz: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MemoryProfile {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub bandwidth_mbs: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct StorageProfile {
    pub io_uring: bool,
    pub o_direct: bool,
    pub read_speed_mbs: f64,
    pub write_speed_mbs: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct VulkanDeviceProfile {
    pub name: String,
    pub vram_bytes: u64,
    pub pcie_info: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct GpuProfile {
    pub devices: Vec<VulkanDeviceProfile>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct NumaNode {
    pub id: u32,
    pub cpu_list: String,
    pub memory_mb: u64,
    pub distances: Vec<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct NumaProfile {
    pub node_count: u32,
    pub nodes: Vec<NumaNode>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct HardwareProfile {
    pub cpu: CpuProfile,
    pub memory: MemoryProfile,
    pub storage: StorageProfile,
    pub gpu: GpuProfile,
    #[serde(default)]
    pub numa: NumaProfile,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct JobRequest {
    pub hardware: HardwareProfile,
    pub project: String,
    pub git_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
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
                ..Default::default()
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
            ..Default::default()
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
                ..Default::default()
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
            ..Default::default()
        };

        let request = JobRequest {
            hardware: profile,
            project: "https://github.com/example/project.git".to_string(),
            git_ref: "main".to_string(),
            binary: None,
            package: None,
            target: None,
        };

        let serialized = serde_json::to_string(&request).expect("Failed to serialize JobRequest");
        let deserialized: JobRequest = serde_json::from_str(&serialized).expect("Failed to deserialize JobRequest");

        assert_eq!(request, deserialized);
    }

    #[test]
    fn test_job_request_binary_field_serialization() {
        let profile = HardwareProfile {
            cpu: CpuProfile {
                flags: vec![],
                cache_topology: "".to_string(),
                core_count: 4,
                ..Default::default()
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
            ..Default::default()
        };

        // Case 1: binary = None
        let req_none = JobRequest {
            hardware: profile.clone(),
            project: "myproj".to_string(),
            git_ref: "main".to_string(),
            binary: None,
            package: None,
            target: None,
        };
        let serialized_none = serde_json::to_string(&req_none).unwrap();
        assert!(!serialized_none.contains("\"binary\""));

        // Case 2: binary = Some("server")
        let req_some = JobRequest {
            hardware: profile.clone(),
            project: "myproj".to_string(),
            git_ref: "main".to_string(),
            binary: Some("server".to_string()),
            package: None,
            target: None,
        };
        let serialized_some = serde_json::to_string(&req_some).unwrap();
        assert!(serialized_some.contains("\"binary\":\"server\""));

        // Case 3: Deserializing old JSON without "binary" key
        let old_json = r#"{
            "project": "myproj",
            "git_ref": "main",
            "hardware": {
                "cpu": {"flags":[], "cache_topology":"", "core_count":4},
                "memory": {"total_bytes":8589934592, "available_bytes":4294967296, "bandwidth_mbs":12000.0},
                "storage": {"io_uring":false, "o_direct":false, "read_speed_mbs":500.0, "write_speed_mbs":450.0},
                "gpu": {"devices":[]}
            }
        }"#;
        let deserialized: JobRequest = serde_json::from_str(old_json).unwrap();
        assert_eq!(deserialized.binary, None);
        assert_eq!(deserialized.package, None);
        assert_eq!(deserialized.project, "myproj");
    }

    #[test]
    fn test_job_request_package_field_serialization() {
        let profile = HardwareProfile {
            cpu: CpuProfile {
                flags: vec![],
                cache_topology: "".to_string(),
                core_count: 4,
                ..Default::default()
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
            ..Default::default()
        };

        // 1. package: None serializes without "package" key in JSON
        let req_none = JobRequest {
            hardware: profile.clone(),
            project: "myproj".to_string(),
            git_ref: "main".to_string(),
            binary: Some("server".to_string()),
            package: None,
            target: None,
        };
        let serialized_none = serde_json::to_string(&req_none).unwrap();
        assert!(!serialized_none.contains("\"package\""));

        // 2. package: Some("server") serializes with "package":"server"
        let req_some = JobRequest {
            hardware: profile.clone(),
            project: "myproj".to_string(),
            git_ref: "main".to_string(),
            binary: None,
            package: Some("server".to_string()),
            target: None,
        };
        let serialized_some = serde_json::to_string(&req_some).unwrap();
        assert!(serialized_some.contains("\"package\":\"server\""));

        // 3. Old JSON without "package" key deserializes to package: None
        let old_json = r#"{
            "project": "myproj",
            "git_ref": "main",
            "hardware": {
                "cpu": {"flags":[], "cache_topology":"", "core_count":4},
                "memory": {"total_bytes":8589934592, "available_bytes":4294967296, "bandwidth_mbs":12000.0},
                "storage": {"io_uring":false, "o_direct":false, "read_speed_mbs":500.0, "write_speed_mbs":450.0},
                "gpu": {"devices":[]}
            }
        }"#;
        let deserialized: JobRequest = serde_json::from_str(old_json).unwrap();
        assert_eq!(deserialized.package, None);
        assert_eq!(deserialized.binary, None);

        // 4. package: None and binary: None both absent -> both fields missing
        let req_both_none = JobRequest {
            hardware: profile.clone(),
            project: "myproj".to_string(),
            git_ref: "main".to_string(),
            binary: None,
            package: None,
            target: None,
        };
        let serialized_both_none = serde_json::to_string(&req_both_none).unwrap();
        assert!(!serialized_both_none.contains("\"binary\""));
        assert!(!serialized_both_none.contains("\"package\""));
    }

    #[test]
    fn test_job_request_target_field_serialization() {
        let profile = HardwareProfile {
            cpu: CpuProfile {
                flags: vec![],
                cache_topology: "".to_string(),
                core_count: 4,
                ..Default::default()
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
            ..Default::default()
        };

        // 8. target: None serializes without "target" key in JSON
        let req_none = JobRequest {
            hardware: profile.clone(),
            project: "myproj".to_string(),
            git_ref: "main".to_string(),
            binary: None,
            package: None,
            target: None,
        };
        let serialized_none = serde_json::to_string(&req_none).unwrap();
        assert!(!serialized_none.contains("\"target\""));

        // 9. target: Some("aarch64-unknown-linux-gnu") serializes with "target":"aarch64-unknown-linux-gnu"
        let req_some = JobRequest {
            hardware: profile.clone(),
            project: "myproj".to_string(),
            git_ref: "main".to_string(),
            binary: None,
            package: None,
            target: Some("aarch64-unknown-linux-gnu".to_string()),
        };
        let serialized_some = serde_json::to_string(&req_some).unwrap();
        assert!(serialized_some.contains("\"target\":\"aarch64-unknown-linux-gnu\""));

        // 10. Old JSON without "target" key deserializes with target: None
        let old_json = r#"{
            "project": "myproj",
            "git_ref": "main",
            "hardware": {
                "cpu": {"flags":[], "cache_topology":"", "core_count":4},
                "memory": {"total_bytes":8589934592, "available_bytes":4294967296, "bandwidth_mbs":12000.0},
                "storage": {"io_uring":false, "o_direct":false, "read_speed_mbs":500.0, "write_speed_mbs":450.0},
                "gpu": {"devices":[]}
            }
        }"#;
        let deserialized: JobRequest = serde_json::from_str(old_json).unwrap();
        assert_eq!(deserialized.target, None);
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

    #[test]
    fn test_hardware_profile_new_fields_roundtrip() {
        let profile = HardwareProfile {
            cpu: CpuProfile {
                flags: vec!["avx2".to_string()],
                cache_topology: "L1:32KB".to_string(),
                core_count: 4,
                cache_line_size: Some(64),
                kernel_version: Some("Linux 5.15.0".to_string()),
                cpu_base_freq_mhz: Some(2500),
                cpu_max_freq_mhz: Some(4200),
            },
            memory: MemoryProfile {
                total_bytes: 8589934592,
                available_bytes: 4294967296,
                bandwidth_mbs: 12000.0,
            },
            storage: StorageProfile {
                io_uring: true,
                o_direct: true,
                read_speed_mbs: 1500.0,
                write_speed_mbs: 1200.0,
            },
            gpu: GpuProfile { devices: vec![] },
            numa: NumaProfile {
                node_count: 2,
                nodes: vec![
                    NumaNode {
                        id: 0,
                        cpu_list: "0-3".to_string(),
                        memory_mb: 4096,
                        distances: vec![10, 20],
                    },
                    NumaNode {
                        id: 1,
                        cpu_list: "4-7".to_string(),
                        memory_mb: 4096,
                        distances: vec![20, 10],
                    },
                ],
            },
        };

        let serialized = serde_json::to_string(&profile).expect("Failed to serialize HardwareProfile");
        let deserialized: HardwareProfile = serde_json::from_str(&serialized).expect("Failed to deserialize HardwareProfile");

        assert_eq!(profile, deserialized);
    }

    #[test]
    fn test_hardware_profile_backward_compatibility() {
        let old_json = r#"{
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
                "io_uring": true,
                "o_direct": true,
                "read_speed_mbs": 1500.0,
                "write_speed_mbs": 1200.0
            },
            "gpu": {
                "devices": []
            }
        }"#;

        let deserialized: HardwareProfile = serde_json::from_str(old_json).expect("Failed to deserialize old JSON");
        assert_eq!(deserialized.cpu.cache_line_size, None);
        assert_eq!(deserialized.cpu.kernel_version, None);
        assert_eq!(deserialized.cpu.cpu_base_freq_mhz, None);
        assert_eq!(deserialized.cpu.cpu_max_freq_mhz, None);
        assert_eq!(deserialized.numa.node_count, 0);
        assert!(deserialized.numa.nodes.is_empty());
    }
}
