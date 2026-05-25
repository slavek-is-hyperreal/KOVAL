pub const SUPPORTED_TARGETS: &[&str] = &[
    "aarch64-unknown-linux-gnu",
    "armv7-unknown-linux-gnueabihf",
    "x86_64-unknown-linux-musl",
];

/// Checks if the target triple is supported by Koval.
pub fn is_supported(triple: &str) -> bool {
    SUPPORTED_TARGETS.contains(&triple)
}

/// Returns the cargo target linker environment variable name and the path/command of the cross linker
/// for a supported target triple.
pub fn linker_env_for_target(triple: &str) -> Option<(String, String)> {
    if !is_supported(triple) {
        return None;
    }

    let linker_bin = match triple {
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu-gcc",
        "armv7-unknown-linux-gnueabihf" => "arm-linux-gnueabihf-gcc",
        "x86_64-unknown-linux-musl" => "musl-gcc",
        _ => return None,
    };

    let env_var = format!(
        "CARGO_TARGET_{}_LINKER",
        triple.replace('-', "_").to_uppercase()
    );

    Some((env_var, linker_bin.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_supported_aarch64() {
        assert!(is_supported("aarch64-unknown-linux-gnu"));
    }

    #[test]
    fn test_is_supported_musl() {
        assert!(is_supported("x86_64-unknown-linux-musl"));
    }

    #[test]
    fn test_is_supported_unsupported() {
        assert!(!is_supported("wasm32-unknown-unknown"));
    }

    #[test]
    fn test_is_supported_empty() {
        assert!(!is_supported(""));
    }

    #[test]
    fn test_linker_env_aarch64() {
        let res = linker_env_for_target("aarch64-unknown-linux-gnu").unwrap();
        assert_eq!(res.0, "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER");
        assert_eq!(res.1, "aarch64-linux-gnu-gcc");
    }

    #[test]
    fn test_linker_env_musl() {
        let res = linker_env_for_target("x86_64-unknown-linux-musl").unwrap();
        assert_eq!(res.0, "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER");
        assert_eq!(res.1, "musl-gcc");
    }

    #[test]
    fn test_linker_env_unsupported() {
        assert!(linker_env_for_target("wasm32-unknown-unknown").is_none());
    }
}
