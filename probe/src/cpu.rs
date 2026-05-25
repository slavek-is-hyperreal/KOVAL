use std::fs;
use std::thread;
use schema::CpuProfile;

pub fn collect() -> CpuProfile {
    let core_count = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let flags = collect_cpu_flags();
    let cache_topology = collect_cache_topology();

    let cache_line_size = read_cache_line_size("/sys/devices/system/cpu/cpu0/cache/index0/coherency_line_size");
    let kernel_version = read_kernel_version("/proc/version");
    let cpu_base_freq_mhz = read_cpu_base_freq(
        "/sys/devices/system/cpu/cpu0/cpufreq/base_frequency",
        "/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_min_freq",
    );
    let cpu_max_freq_mhz = read_cpu_max_freq("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq");

    CpuProfile {
        flags,
        cache_topology,
        core_count,
        cache_line_size,
        kernel_version,
        cpu_base_freq_mhz,
        cpu_max_freq_mhz,
    }
}

fn collect_cpu_flags() -> Vec<String> {
    let mut flags = Vec::new();
    if let Ok(content) = fs::read_to_string("/proc/cpuinfo") {
        for line in content.lines() {
            if line.starts_with("flags") || line.starts_with("Features") {
                if let Some(pos) = line.find(':') {
                    let parts = line[pos + 1..]
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>();
                    flags = parts;
                    break;
                }
            }
        }
    }
    flags
}

fn collect_cache_topology() -> String {
    let mut topology = Vec::new();
    for index in 0..4 {
        let cache_dir = format!("/sys/devices/system/cpu/cpu0/cache/index{}", index);
        let level_path = format!("{}/level", cache_dir);
        let type_path = format!("{}/type", cache_dir);
        let size_path = format!("{}/size", cache_dir);

        if let (Ok(level), Ok(ty), Ok(size)) = (
            fs::read_to_string(level_path),
            fs::read_to_string(type_path),
            fs::read_to_string(size_path),
        ) {
            let level = level.trim();
            let ty = ty.trim();
            let size = size.trim();
            let ty_short = match ty {
                "Data" => "d",
                "Instruction" => "i",
                _ => "",
            };
            topology.push(format!("L{}{}:{}", level, ty_short, size));
        }
    }

    if topology.is_empty() {
        "Unknown".to_string()
    } else {
        topology.join(", ")
    }
}

fn read_cache_line_size(path: &str) -> Option<u64> {
    let content = fs::read_to_string(path).ok()?;
    content.trim().parse::<u64>().ok()
}

fn read_kernel_version(path: &str) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    content.lines().next().map(|line| line.trim().to_string())
}

fn read_cpu_base_freq(base_path: &str, fallback_path: &str) -> Option<u64> {
    if let Ok(content) = fs::read_to_string(base_path) {
        if let Ok(khz) = content.trim().parse::<u64>() {
            return Some(khz / 1000);
        }
    }
    if let Ok(content) = fs::read_to_string(fallback_path) {
        if let Ok(khz) = content.trim().parse::<u64>() {
            return Some(khz / 1000);
        }
    }
    None
}

fn read_cpu_max_freq(path: &str) -> Option<u64> {
    let content = fs::read_to_string(path).ok()?;
    let khz = content.trim().parse::<u64>().ok()?;
    Some(khz / 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_temp_file(name: &str, content: &str) -> String {
        let temp_dir = std::env::temp_dir().join("koval_tests");
        let _ = fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join(name);
        fs::write(&file_path, content).unwrap();
        file_path.to_string_lossy().to_string()
    }

    #[test]
    fn test_read_cache_line_size() {
        // Valid
        let path = write_temp_file("coherency_line_size_valid", "64\n");
        assert_eq!(read_cache_line_size(&path), Some(64));

        // Invalid
        let path_invalid = write_temp_file("coherency_line_size_invalid", "abc\n");
        assert_eq!(read_cache_line_size(&path_invalid), None);

        // Missing
        assert_eq!(read_cache_line_size("/non/existing/path/cache_line"), None);
    }

    #[test]
    fn test_read_kernel_version() {
        // Valid
        let path = write_temp_file("version_valid", "Linux version 5.15.0-72-generic (buildd@boszo) (gcc version 11.3.0) #79-Ubuntu SMP \nAnother line");
        assert_eq!(read_kernel_version(&path), Some("Linux version 5.15.0-72-generic (buildd@boszo) (gcc version 11.3.0) #79-Ubuntu SMP".to_string()));

        // Missing
        assert_eq!(read_kernel_version("/non/existing/path/version"), None);
    }

    #[test]
    fn test_read_cpu_base_freq() {
        let path_base = write_temp_file("base_freq", "2400000\n");
        let path_fallback = write_temp_file("min_freq", "1000000\n");

        // Both present: uses base
        assert_eq!(read_cpu_base_freq(&path_base, &path_fallback), Some(2400));

        // Base missing, fallback present: uses fallback
        assert_eq!(read_cpu_base_freq("/non/existing/base", &path_fallback), Some(1000));

        // Base invalid, fallback present: uses fallback
        let path_invalid_base = write_temp_file("invalid_base_freq", "invalid\n");
        assert_eq!(read_cpu_base_freq(&path_invalid_base, &path_fallback), Some(1000));

        // Both missing
        assert_eq!(read_cpu_base_freq("/non/existing/base", "/non/existing/min"), None);
    }

    #[test]
    fn test_read_cpu_max_freq() {
        // Valid
        let path = write_temp_file("max_freq_valid", "4200000\n");
        assert_eq!(read_cpu_max_freq(&path), Some(4200));

        // Invalid
        let path_invalid = write_temp_file("max_freq_invalid", "xyz\n");
        assert_eq!(read_cpu_max_freq(&path_invalid), None);

        // Missing
        assert_eq!(read_cpu_max_freq("/non/existing/path/max_freq"), None);
    }
}
