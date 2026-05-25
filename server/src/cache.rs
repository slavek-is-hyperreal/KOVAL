use sha2::{Digest, Sha256};

/// Computes the unique cache key for a build request based on hardware, project, git reference, binary target, and package.
pub fn compute_cache_key(hardware_json: &str, project: &str, git_ref: &str, binary: Option<&str>, package: Option<&str>) -> String {
    let binary_str = match binary {
        Some(b) => format!("some:{}", b),
        None => "none".to_string(),
    };
    let package_str = match package {
        Some(p) => format!("some:{}", p),
        None => "none".to_string(),
    };
    let concatenated = format!("{}|{}|{}|{}|{}", hardware_json, project, git_ref, binary_str, package_str);
    
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
        
        let key1 = compute_cache_key(hw, proj, git_ref, None, None);
        let key2 = compute_cache_key(hw, proj, git_ref, None, None);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_different_hardware() {
        let hw1 = r#"{"cpu":{"flags":["avx2"]}}"#;
        let hw2 = r#"{"cpu":{"flags":[]}}"#;
        let proj = "https://github.com/example/project";
        let git_ref = "main";
        
        let key1 = compute_cache_key(hw1, proj, git_ref, None, None);
        let key2 = compute_cache_key(hw2, proj, git_ref, None, None);
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_cache_key_binary_variations() {
        let hw = r#"{"cpu":{"flags":["avx2"]}}"#;
        let proj = "https://github.com/example/project";
        let git_ref = "main";
        
        let key_none = compute_cache_key(hw, proj, git_ref, None, None);
        let key_empty = compute_cache_key(hw, proj, git_ref, Some(""), None);
        let key_server = compute_cache_key(hw, proj, git_ref, Some("server"), None);
        
        assert_ne!(key_none, key_server);
        assert_ne!(key_empty, key_server);
        assert_ne!(key_none, key_empty);
    }

    #[test]
    fn test_cache_key_package_variations() {
        let hw = r#"{"cpu":{"flags":["avx2"]}}"#;
        let proj = "https://github.com/example/project";
        let git_ref = "main";
        
        // 7. package: None vs package: Some("server") -> different keys
        let key_none = compute_cache_key(hw, proj, git_ref, None, None);
        let key_server = compute_cache_key(hw, proj, git_ref, None, Some("server"));
        assert_ne!(key_none, key_server);

        // 8. binary: None vs binary: Some("probe") -> different keys
        let key_bin_none = compute_cache_key(hw, proj, git_ref, None, None);
        let key_bin_probe = compute_cache_key(hw, proj, git_ref, Some("probe"), None);
        assert_ne!(key_bin_none, key_bin_probe);

        // 9. package: None and package: Some("") -> different keys
        let key_empty = compute_cache_key(hw, proj, git_ref, None, Some(""));
        assert_ne!(key_none, key_empty);
    }
}
