use std::fs;
use std::hint::black_box;
use std::time::Instant;
use schema::MemoryProfile;

pub fn collect() -> MemoryProfile {
    let (total_bytes, available_bytes) = collect_ram_sizes();
    let bandwidth_mbs = measure_ram_bandwidth();

    MemoryProfile {
        total_bytes,
        available_bytes,
        bandwidth_mbs,
    }
}

fn collect_ram_sizes() -> (u64, u64) {
    let mut total = 0;
    let mut available = 0;

    if let Ok(content) = fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if line.starts_with("MemTotal:") {
                total = parse_meminfo_line(line);
            } else if line.starts_with("MemAvailable:") {
                available = parse_meminfo_line(line);
            }
        }
    }

    (total, available)
}

fn parse_meminfo_line(line: &str) -> u64 {
    // Format: "MemTotal:       32800364 kB"
    let parts = line.split_whitespace().collect::<Vec<_>>();
    if parts.len() >= 2 {
        if let Ok(kb) = parts[1].parse::<u64>() {
            return kb * 1024; // convert to bytes
        }
    }
    0
}

fn measure_ram_bandwidth() -> f64 {
    // Benchmark memory copy: allocate 32MB (4,194,304 of u64s)
    let size = 4 * 1024 * 1024;
    let src = vec![1u64; size];
    let mut dest = vec![0u64; size];

    let start = Instant::now();
    let iterations = 50;

    for _ in 0..iterations {
        // Copy using std::hint::black_box to prevent compiler optimization
        let s = black_box(&src[..]);
        let d = black_box(&mut dest[..]);
        d.copy_from_slice(s);
    }

    let elapsed = start.elapsed().as_secs_f64();
    if elapsed == 0.0 {
        return 0.0;
    }

    // Total bytes transferred: read 32MB + write 32MB = 64MB per iteration
    let bytes_transferred = (size * 8 * 2 * iterations) as f64;
    let mb_transferred = bytes_transferred / (1024.0 * 1024.0);

    mb_transferred / elapsed
}
