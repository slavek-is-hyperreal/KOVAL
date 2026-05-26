use std::path::Path;
use std::process::Command;

pub fn instrument_flags(job_id: &str, base_dir: &Path) -> Vec<String> {
    vec![
        "-C".to_string(),
        format!("profile-generate={}/pgo/{}", base_dir.display(), job_id),
    ]
}

pub fn optimize_flags(profdata_path: &Path) -> Vec<String> {
    vec![
        "-C".to_string(),
        format!("profile-use={}", profdata_path.display()),
        "-C".to_string(),
        "llvm-args=-pgo-warn-missing-function".to_string(),
    ]
}

pub fn merge_command(profiles_dir: &Path, output_path: &Path) -> Result<Command, std::io::Error> {
    let mut cmd = Command::new("llvm-profdata");
    cmd.arg("merge");

    let mut has_files = false;
    for entry in std::fs::read_dir(profiles_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("profraw") {
            cmd.arg(path);
            has_files = true;
        }
    }

    if !has_files {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No .profraw files found in directory",
        ));
    }

    cmd.arg("--output");
    cmd.arg(output_path);

    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn test_instrument_flags() {
        let flags = instrument_flags("abc123", Path::new("/artifacts"));
        assert!(flags.contains(&"-C".to_string()));
        assert!(flags.contains(&"profile-generate=/artifacts/pgo/abc123".to_string()));
    }

    #[test]
    fn test_optimize_flags() {
        let flags = optimize_flags(Path::new("/artifacts/pgo/abc123/merged.profdata"));
        assert!(flags.contains(&"-C".to_string()));
        assert!(flags.contains(&"profile-use=/artifacts/pgo/abc123/merged.profdata".to_string()));
        assert!(flags.contains(&"llvm-args=-pgo-warn-missing-function".to_string()));
    }

    #[test]
    fn test_merge_command() {
        // Create a temporary directory and some dummy .profraw files
        let tmp_dir = std::env::temp_dir().join("test_merge_command_dir");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        
        let file1 = tmp_dir.join("1.profraw");
        let file2 = tmp_dir.join("2.profraw");
        let other_file = tmp_dir.join("other.txt");
        
        File::create(&file1).unwrap();
        File::create(&file2).unwrap();
        File::create(&other_file).unwrap();
        
        let output = tmp_dir.join("merged.profdata");
        
        let cmd = merge_command(&tmp_dir, &output).unwrap();
        assert_eq!(cmd.get_program().to_str().unwrap(), "llvm-profdata");
        
        let args: Vec<String> = cmd.get_args().map(|a| a.to_str().unwrap().to_string()).collect();
        assert_eq!(args[0], "merge");
        assert!(args.contains(&file1.to_str().unwrap().to_string()));
        assert!(args.contains(&file2.to_str().unwrap().to_string()));
        assert!(!args.contains(&other_file.to_str().unwrap().to_string()));
        
        // Output flag and path
        assert!(args.contains(&"--output".to_string()));
        assert!(args.contains(&output.to_str().unwrap().to_string()));
        
        // Clean up
        std::fs::remove_dir_all(&tmp_dir).unwrap();
    }
}
