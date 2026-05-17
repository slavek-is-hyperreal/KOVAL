use std::fs;
use std::thread;
use schema::CpuProfile;

pub fn collect() -> CpuProfile {
    let core_count = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let flags = collect_cpu_flags();
    let cache_topology = collect_cache_topology();

    CpuProfile {
        flags,
        cache_topology,
        core_count,
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
