use sha2::{Digest, Sha256};
use schema::HardwareProfile;

/// Computes the unique cache key for a build request based on hardware, project, git reference, binary target, package, and target.
pub fn compute_cache_key(
    hardware_json: &str,
    project: &str,
    git_ref: &str,
    binary: Option<&str>,
    package: Option<&str>,
    target: Option<&str>,
) -> String {
    // Attempt to parse and normalize the hardware json, fallback to the original if parsing fails
    let normalized_hw_json = if let Ok(mut profile) = serde_json::from_str::<HardwareProfile>(hardware_json) {
        // Normalize dynamic / benchmarking fields
        profile.memory.available_bytes = 0;
        profile.memory.bandwidth_mbs = 0.0;
        profile.memory.latency_ns_l1 = None;
        profile.memory.latency_ns_l2 = None;
        profile.memory.latency_ns_l3 = None;
        profile.memory.latency_ns_ram = None;

        profile.storage.read_speed_mbs = 0.0;
        profile.storage.write_speed_mbs = 0.0;

        serde_json::to_string(&profile).unwrap_or_else(|_| hardware_json.to_string())
    } else {
        hardware_json.to_string()
    };

    let binary_str = match binary {
        Some(b) => format!("some:{}", b),
        None => "none".to_string(),
    };
    let package_str = match package {
        Some(p) => format!("some:{}", p),
        None => "none".to_string(),
    };
    let target_str = match target {
        Some(t) => format!("some:{}", t),
        None => "none".to_string(),
    };
    let concatenated = format!(
        "{}|{}|{}|{}|{}|{}",
        normalized_hw_json, project, git_ref, binary_str, package_str, target_str
    );
    
    let mut hasher = Sha256::new();
    hasher.update(concatenated.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_consistency() {
        let hw = r#"{"cpu":{"flags":["avx2"]}}"#;
        let proj = "https://github.com/example/project";
        let git_ref = "main";
        
        let key1 = compute_cache_key(hw, proj, git_ref, None, None, None);
        let key2 = compute_cache_key(hw, proj, git_ref, None, None, None);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_different_hardware() {
        let hw1 = r#"{"cpu":{"flags":["avx2"]}}"#;
        let hw2 = r#"{"cpu":{"flags":[]}}"#;
        let proj = "https://github.com/example/project";
        let git_ref = "main";
        
        let key1 = compute_cache_key(hw1, proj, git_ref, None, None, None);
        let key2 = compute_cache_key(hw2, proj, git_ref, None, None, None);
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_cache_key_binary_variations() {
        let hw = r#"{"cpu":{"flags":["avx2"]}}"#;
        let proj = "https://github.com/example/project";
        let git_ref = "main";
        
        let key_none = compute_cache_key(hw, proj, git_ref, None, None, None);
        let key_empty = compute_cache_key(hw, proj, git_ref, Some(""), None, None);
        let key_server = compute_cache_key(hw, proj, git_ref, Some("server"), None, None);
        
        assert_ne!(key_none, key_server);
        assert_ne!(key_empty, key_server);
        assert_ne!(key_none, key_empty);
    }

    #[test]
    fn test_cache_key_package_variations() {
        let hw = r#"{"cpu":{"flags":["avx2"]}}"#;
        let proj = "https://github.com/example/project";
        let git_ref = "main";
        
        let key_none = compute_cache_key(hw, proj, git_ref, None, None, None);
        let key_server = compute_cache_key(hw, proj, git_ref, None, Some("server"), None);
        assert_ne!(key_none, key_server);

        let key_bin_none = compute_cache_key(hw, proj, git_ref, None, None, None);
        let key_bin_probe = compute_cache_key(hw, proj, git_ref, Some("probe"), None, None);
        assert_ne!(key_bin_none, key_bin_probe);

        let key_empty = compute_cache_key(hw, proj, git_ref, None, Some(""), None);
        assert_ne!(key_none, key_empty);
    }

    #[test]
    fn test_cache_key_target_variations() {
        let hw = r#"{"cpu":{"flags":["avx2"]}}"#;
        let proj = "https://github.com/example/project";
        let git_ref = "main";

        // 11. Same inputs including target: None -> same key
        let key1 = compute_cache_key(hw, proj, git_ref, None, None, None);
        let key2 = compute_cache_key(hw, proj, git_ref, None, None, None);
        assert_eq!(key1, key2);

        // 12. target: None vs target: Some("aarch64-unknown-linux-gnu") -> different keys
        let key_aarch64 = compute_cache_key(hw, proj, git_ref, None, None, Some("aarch64-unknown-linux-gnu"));
        assert_ne!(key1, key_aarch64);

        // 13. target: None vs target: Some("") -> different keys
        let key_empty = compute_cache_key(hw, proj, git_ref, None, None, Some(""));
        assert_ne!(key1, key_empty);
    }
}
