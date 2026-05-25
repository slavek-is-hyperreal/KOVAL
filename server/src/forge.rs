use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use schema::HardwareProfile;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ForgeRule {
    pub cpu_flags: Option<Vec<String>>,
    pub min_cores: Option<usize>,
    pub min_memory_gb: Option<f64>,
    pub min_memory_bandwidth: Option<f64>,
    pub require_io_uring: Option<bool>,
    pub require_o_direct: Option<bool>,
    pub min_storage_read_mbs: Option<f64>,
    pub min_gpu_vram_gb: Option<f64>,

    pub min_cpu_base_freq_mhz: Option<u64>,
    pub min_cpu_max_freq_mhz: Option<u64>,
    pub min_numa_nodes: Option<u32>,
    pub require_cache_line_size: Option<u64>,

    pub rustflags: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub features: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct KovalToml {
    #[serde(default)]
    pub rules: Vec<ForgeRule>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BuildConfig {
    pub rustflags: String, // space-separated
    pub env: HashMap<String, String>,
    pub features: Vec<String>,
}

/// Pure function: matches HardwareProfile and KovalToml rules to produce BuildConfig
pub fn build_config(hardware: &HardwareProfile, config: &KovalToml) -> BuildConfig {
    let mut collected_flags: Vec<String> = Vec::new();
    let mut collected_env: HashMap<String, String> = HashMap::new();
    let mut collected_features: Vec<String> = Vec::new();

    for rule in &config.rules {
        let mut matched = true;

        // 1. CPU flags match (all specified flags must be present)
        if let Some(ref req_flags) = rule.cpu_flags {
            let cpu_flags = &hardware.cpu.flags;
            if !req_flags.iter().all(|f| cpu_flags.contains(f)) {
                matched = false;
            }
        }

        // 2. Minimum cores
        if let Some(min_cores) = rule.min_cores {
            if hardware.cpu.core_count < min_cores {
                matched = false;
            }
        }

        // 3. Minimum memory in GB
        if let Some(min_mem) = rule.min_memory_gb {
            let total_gb = (hardware.memory.total_bytes as f64) / (1024.0 * 1024.0 * 1024.0);
            if total_gb < min_mem {
                matched = false;
            }
        }

        // 4. Minimum memory bandwidth
        if let Some(min_bw) = rule.min_memory_bandwidth {
            if hardware.memory.bandwidth_mbs < min_bw {
                matched = false;
            }
        }

        // 5. io_uring
        if let Some(req_uring) = rule.require_io_uring {
            if hardware.storage.io_uring != req_uring {
                matched = false;
            }
        }

        // 6. O_DIRECT
        if let Some(req_direct) = rule.require_o_direct {
            if hardware.storage.o_direct != req_direct {
                matched = false;
            }
        }

        // 7. Storage read speed
        if let Some(min_read) = rule.min_storage_read_mbs {
            if hardware.storage.read_speed_mbs < min_read {
                matched = false;
            }
        }

        // 8. GPU VRAM
        if let Some(min_vram) = rule.min_gpu_vram_gb {
            let mut has_matching_gpu = false;
            for dev in &hardware.gpu.devices {
                let vram_gb = (dev.vram_bytes as f64) / (1024.0 * 1024.0 * 1024.0);
                if vram_gb >= min_vram {
                    has_matching_gpu = true;
                    break;
                }
            }
            if !has_matching_gpu {
                matched = false;
            }
        }

        // 9. min_cpu_base_freq_mhz
        if let Some(min_base) = rule.min_cpu_base_freq_mhz {
            match hardware.cpu.cpu_base_freq_mhz {
                Some(freq) if freq >= min_base => {}
                _ => matched = false,
            }
        }

        // 10. min_cpu_max_freq_mhz
        if let Some(min_max) = rule.min_cpu_max_freq_mhz {
            match hardware.cpu.cpu_max_freq_mhz {
                Some(freq) if freq >= min_max => {}
                _ => matched = false,
            }
        }

        // 11. min_numa_nodes
        if let Some(min_nodes) = rule.min_numa_nodes {
            if hardware.numa.node_count < min_nodes {
                matched = false;
            }
        }

        // 12. require_cache_line_size
        if let Some(req_line_size) = rule.require_cache_line_size {
            match hardware.cpu.cache_line_size {
                Some(line_size) if line_size == req_line_size => {}
                _ => matched = false,
            }
        }

        // Apply rule if matched
        if matched {
            if let Some(ref flags) = rule.rustflags {
                collected_flags.extend(flags.iter().cloned());
            }
            if let Some(ref env) = rule.env {
                for (k, v) in env {
                    collected_env.insert(k.clone(), v.clone());
                }
            }
            if let Some(ref features) = rule.features {
                collected_features.extend(features.iter().cloned());
            }
        }
    }

    // De-duplicate features
    collected_features.sort();
    collected_features.dedup();

    BuildConfig {
        rustflags: collected_flags.join(" "),
        env: collected_env,
        features: collected_features,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema::{CpuProfile, MemoryProfile, StorageProfile, GpuProfile, VulkanDeviceProfile};

    fn get_fixture_hardware() -> HardwareProfile {
        HardwareProfile {
            cpu: CpuProfile {
                flags: vec!["avx2".to_string(), "sse4.1".to_string(), "aes".to_string()],
                cache_topology: "L1:32KB".to_string(),
                core_count: 8,
                cache_line_size: Some(64),
                cpu_base_freq_mhz: Some(2500),
                cpu_max_freq_mhz: Some(4200),
                ..Default::default()
            },
            memory: MemoryProfile {
                total_bytes: 17179869184, // 16 GB
                available_bytes: 8589934592,
                bandwidth_mbs: 25000.0,
                ..Default::default()
            },
            storage: StorageProfile {
                io_uring: true,
                o_direct: true,
                read_speed_mbs: 3500.0,
                write_speed_mbs: 3000.0,
            },
            gpu: GpuProfile {
                devices: vec![VulkanDeviceProfile {
                    name: "NVIDIA RTX 4090".to_string(),
                    vram_bytes: 25769803776, // 24 GB
                    pcie_info: None,
                }],
            },
            numa: schema::NumaProfile {
                node_count: 2,
                nodes: vec![
                    schema::NumaNode {
                        id: 0,
                        cpu_list: "0-3".to_string(),
                        memory_mb: 8192,
                        distances: vec![10, 20],
                    },
                    schema::NumaNode {
                        id: 1,
                        cpu_list: "4-7".to_string(),
                        memory_mb: 8192,
                        distances: vec![20, 10],
                    },
                ],
            },
        }
    }

    #[test]
    fn test_forge_avx2_rule_matching() {
        let hardware = get_fixture_hardware();
        let config = KovalToml {
            rules: vec![ForgeRule {
                cpu_flags: Some(vec!["avx2".to_string(), "aes".to_string()]),
                rustflags: Some(vec!["-C".to_string(), "target-feature=+avx2".to_string()]),
                env: Some([("KOVAL_AVX2".to_string(), "1".to_string())].into_iter().collect()),
                features: Some(vec!["avx2-acceleration".to_string()]),
                ..Default::default()
            }],
        };

        let build = build_config(&hardware, &config);
        assert_eq!(build.rustflags, "-C target-feature=+avx2");
        assert_eq!(build.env.get("KOVAL_AVX2").unwrap(), "1");
        assert_eq!(build.features, vec!["avx2-acceleration".to_string()]);
    }

    #[test]
    fn test_forge_min_memory_and_gpu_matching() {
        let hardware = get_fixture_hardware();
        let config = KovalToml {
            rules: vec![
                ForgeRule {
                    min_memory_gb: Some(15.0),
                    min_gpu_vram_gb: Some(20.0),
                    rustflags: Some(vec!["-C".to_string(), "opt-level=3".to_string()]),
                    features: Some(vec!["cuda".to_string()]),
                    ..Default::default()
                },
                ForgeRule {
                    min_gpu_vram_gb: Some(30.0), // Should NOT match (hardware has 24GB)
                    features: Some(vec!["ultra-textures".to_string()]),
                    ..Default::default()
                }
            ],
        };

        let build = build_config(&hardware, &config);
        assert_eq!(build.rustflags, "-C opt-level=3");
        assert_eq!(build.features, vec!["cuda".to_string()]);
    }

    #[test]
    fn test_forge_fallback_when_hardware_field_missing() {
        // Create an empty hardware profile representing minimal/missing properties
        let minimal_hardware = HardwareProfile {
            cpu: CpuProfile {
                flags: vec![],
                cache_topology: "".to_string(),
                core_count: 1,
                ..Default::default()
            },
            memory: MemoryProfile {
                total_bytes: 512 * 1024 * 1024, // 512 MB
                available_bytes: 256 * 1024 * 1024,
                bandwidth_mbs: 1000.0,
                ..Default::default()
            },
            storage: StorageProfile {
                io_uring: false,
                o_direct: false,
                read_speed_mbs: 10.0,
                write_speed_mbs: 10.0,
            },
            gpu: GpuProfile { devices: vec![] }, // Missing GPU
            ..Default::default()
        };

        let config = KovalToml {
            rules: vec![
                ForgeRule {
                    min_gpu_vram_gb: Some(8.0),
                    rustflags: Some(vec!["-C".to_string(), "target-cpu=native".to_string()]),
                    ..Default::default()
                },
                ForgeRule {
                    min_cores: Some(4),
                    features: Some(vec!["parallel".to_string()]),
                    ..Default::default()
                }
            ],
        };

        let build = build_config(&minimal_hardware, &config);
        assert_eq!(build.rustflags, "");
        assert!(build.features.is_empty());
        assert!(build.env.is_empty());
    }

    #[test]
    fn test_forge_min_cpu_base_freq_mhz() {
        let hardware = get_fixture_hardware();

        // Match
        let config_match = KovalToml {
            rules: vec![ForgeRule {
                min_cpu_base_freq_mhz: Some(2000),
                features: Some(vec!["base-freq-ok".to_string()]),
                ..Default::default()
            }],
        };
        let build_match = build_config(&hardware, &config_match);
        assert_eq!(build_match.features, vec!["base-freq-ok".to_string()]);

        // Exclude
        let config_exclude = KovalToml {
            rules: vec![ForgeRule {
                min_cpu_base_freq_mhz: Some(3000),
                features: Some(vec!["base-freq-ok".to_string()]),
                ..Default::default()
            }],
        };
        let build_exclude = build_config(&hardware, &config_exclude);
        assert!(build_exclude.features.is_empty());
    }

    #[test]
    fn test_forge_min_cpu_max_freq_mhz() {
        let hardware = get_fixture_hardware();

        // Match
        let config_match = KovalToml {
            rules: vec![ForgeRule {
                min_cpu_max_freq_mhz: Some(4000),
                features: Some(vec!["max-freq-ok".to_string()]),
                ..Default::default()
            }],
        };
        let build_match = build_config(&hardware, &config_match);
        assert_eq!(build_match.features, vec!["max-freq-ok".to_string()]);

        // Exclude
        let config_exclude = KovalToml {
            rules: vec![ForgeRule {
                min_cpu_max_freq_mhz: Some(5000),
                features: Some(vec!["max-freq-ok".to_string()]),
                ..Default::default()
            }],
        };
        let build_exclude = build_config(&hardware, &config_exclude);
        assert!(build_exclude.features.is_empty());
    }

    #[test]
    fn test_forge_min_numa_nodes() {
        let hardware = get_fixture_hardware();

        // Match
        let config_match = KovalToml {
            rules: vec![ForgeRule {
                min_numa_nodes: Some(2),
                features: Some(vec!["numa-ok".to_string()]),
                ..Default::default()
            }],
        };
        let build_match = build_config(&hardware, &config_match);
        assert_eq!(build_match.features, vec!["numa-ok".to_string()]);

        // Exclude
        let config_exclude = KovalToml {
            rules: vec![ForgeRule {
                min_numa_nodes: Some(3),
                features: Some(vec!["numa-ok".to_string()]),
                ..Default::default()
            }],
        };
        let build_exclude = build_config(&hardware, &config_exclude);
        assert!(build_exclude.features.is_empty());
    }

    #[test]
    fn test_forge_require_cache_line_size() {
        let hardware = get_fixture_hardware();

        // Match
        let config_match = KovalToml {
            rules: vec![ForgeRule {
                require_cache_line_size: Some(64),
                features: Some(vec!["cache-line-ok".to_string()]),
                ..Default::default()
            }],
        };
        let build_match = build_config(&hardware, &config_match);
        assert_eq!(build_match.features, vec!["cache-line-ok".to_string()]);

        // Exclude
        let config_exclude = KovalToml {
            rules: vec![ForgeRule {
                require_cache_line_size: Some(128),
                features: Some(vec!["cache-line-ok".to_string()]),
                ..Default::default()
            }],
        };
        let build_exclude = build_config(&hardware, &config_exclude);
        assert!(build_exclude.features.is_empty());
    }

    #[test]
    fn test_forge_backward_compatibility() {
        // Legacies: new fields on CpuProfile and Numa are default/None
        let legacy_hardware = HardwareProfile {
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
                ..Default::default()
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

        // Rule specifying CPU base frequency should NOT match because legacy is None
        let config = KovalToml {
            rules: vec![
                ForgeRule {
                    min_cpu_base_freq_mhz: Some(1000),
                    features: Some(vec!["base-ok".to_string()]),
                    ..Default::default()
                },
                ForgeRule {
                    min_numa_nodes: Some(1),
                    features: Some(vec!["numa-ok".to_string()]),
                    ..Default::default()
                },
                ForgeRule {
                    cpu_flags: Some(vec!["avx2".to_string()]),
                    features: Some(vec!["avx2-ok".to_string()]),
                    ..Default::default()
                }
            ],
        };

        let build = build_config(&legacy_hardware, &config);
        assert_eq!(build.features, vec!["avx2-ok".to_string()]);
    }
}
