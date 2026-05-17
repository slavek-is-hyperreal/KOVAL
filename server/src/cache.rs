use sha2::{Digest, Sha256};

/// Computes the unique cache key for a build request based on hardware, project, git reference, and binary target.
pub fn compute_cache_key(hardware_json: &str, project: &str, git_ref: &str, binary: Option<&str>) -> String {
    let binary_str = binary.unwrap_or("");
    let concatenated = format!("{}|{}|{}|{}", hardware_json, project, git_ref, binary_str);
    
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
        
        let key1 = compute_cache_key(hw, proj, git_ref, None);
        let key2 = compute_cache_key(hw, proj, git_ref, None);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_different_hardware() {
        let hw1 = r#"{"cpu":{"flags":["avx2"]}}"#;
        let hw2 = r#"{"cpu":{"flags":[]}}"#;
        let proj = "https://github.com/example/project";
        let git_ref = "main";
        
        let key1 = compute_cache_key(hw1, proj, git_ref, None);
        let key2 = compute_cache_key(hw2, proj, git_ref, None);
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_cache_key_binary_variations() {
        let hw = r#"{"cpu":{"flags":["avx2"]}}"#;
        let proj = "https://github.com/example/project";
        let git_ref = "main";
        
        let key_none = compute_cache_key(hw, proj, git_ref, None);
        let key_empty = compute_cache_key(hw, proj, git_ref, Some(""));
        let key_server = compute_cache_key(hw, proj, git_ref, Some("server"));
        
        assert_ne!(key_none, key_server);
        assert_ne!(key_empty, key_server);
    }
}
