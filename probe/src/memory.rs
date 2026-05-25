use std::fs;
use std::hint::black_box;
use std::time::Instant;
use schema::MemoryProfile;

pub fn collect() -> MemoryProfile {
    let (total_bytes, available_bytes) = collect_ram_sizes();
    let bandwidth_mbs = measure_ram_bandwidth();

    let latency_ns_l1 = measure_latency_ns(32 * 1024, 10000);
    let latency_ns_l2 = measure_latency_ns(256 * 1024, 5000);
    let latency_ns_l3 = if available_bytes == 0 || available_bytes > 32 * 1024 * 1024 {
        measure_latency_ns(8 * 1024 * 1024, 1000)
    } else {
        None
    };
    let latency_ns_ram = if available_bytes == 0 || available_bytes > 384 * 1024 * 1024 {
        measure_latency_ns(256 * 1024 * 1024, 100)
    } else {
        None
    };

    MemoryProfile {
        total_bytes,
        available_bytes,
        bandwidth_mbs,
        latency_ns_l1,
        latency_ns_l2,
        latency_ns_l3,
        latency_ns_ram,
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

pub fn build_pointer_chase_chain(buf: &mut Vec<usize>) -> usize {
    let n = buf.len();
    if n == 0 {
        return 0;
    }
    if n == 1 {
        buf[0] = 0;
        return 1;
    }

    // Create indices 1..n
    let mut indices: Vec<usize> = (1..n).collect();

    // Shuffle indices using a simple LCG-based random generator
    struct SimpleRng {
        state: u64,
    }
    impl SimpleRng {
        fn next_u64(&mut self) -> u64 {
            self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.state
        }
        fn gen_range(&mut self, limit: usize) -> usize {
            if limit == 0 {
                return 0;
            }
            (self.next_u64() as usize) % limit
        }
    }

    let mut rng = SimpleRng { state: 0x517cc1b727220a95u64 };
    for i in (1..indices.len()).rev() {
        let j = rng.gen_range(i + 1);
        indices.swap(i, j);
    }

    // Build cycle: 0 -> indices[0] -> indices[1] -> ... -> indices[n-2] -> 0
    buf[0] = indices[0];
    for i in 0..n-2 {
        buf[indices[i]] = indices[i+1];
    }
    buf[indices[n-2]] = 0;

    n
}

pub fn measure_latency_ns(working_set_bytes: usize, iterations: usize) -> Option<f64> {
    if iterations == 0 {
        return None;
    }
    let size_of_usize = std::mem::size_of::<usize>();
    let num_elements = working_set_bytes / size_of_usize;
    if num_elements == 0 {
        return None;
    }

    let mut buf = vec![0; num_elements];
    let chain_length = build_pointer_chase_chain(&mut buf);
    if chain_length == 0 {
        return None;
    }

    // Warm up
    let mut curr = 0;
    let warm_up_steps = chain_length.min(100_000);
    for _ in 0..warm_up_steps {
        curr = black_box(buf[curr]);
    }

    // Timed loop
    let total_steps = (iterations * chain_length).min(1_000_000);
    let start = Instant::now();
    let mut curr = 0;
    for _ in 0..total_steps {
        curr = black_box(buf[curr]);
    }
    let elapsed = start.elapsed();
    black_box(curr); // Prevent compiler from optimizing out the loop

    let elapsed_ns = elapsed.as_nanos() as f64;
    if elapsed_ns == 0.0 {
        return None;
    }

    Some(elapsed_ns / (total_steps as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_pointer_chase_chain_length_8() {
        let mut buf = vec![0; 8];
        let chain_length = build_pointer_chase_chain(&mut buf);
        assert_eq!(chain_length, 8);

        let mut visited = vec![false; 8];
        let mut curr = 0;
        for _ in 0..8 {
            assert!(curr < 8, "Index out of bounds");
            assert!(!visited[curr], "Cycle is smaller than expected, visited index twice");
            visited[curr] = true;
            curr = buf[curr];
        }
        assert_eq!(curr, 0, "Chain did not return to start index 0");
        assert!(visited.iter().all(|&v| v), "Not all elements in buffer were visited");
    }

    #[test]
    fn test_build_pointer_chase_chain_length_1() {
        let mut buf = vec![0; 1];
        let chain_length = build_pointer_chase_chain(&mut buf);
        assert_eq!(chain_length, 1);
        assert_eq!(buf[0], 0, "Self-loop at index 0 failed");
    }

    #[test]
    fn test_measure_latency_ns_sanity() {
        let latency = measure_latency_ns(4096, 10);
        assert!(latency.is_some());
        let val = latency.unwrap();
        assert!(val > 0.0, "Latency must be positive");
        assert!(val < 10000.0, "Latency is unreasonably high (>= 10us)");
    }

    #[test]
    fn test_measure_latency_ns_edge_case() {
        let latency = measure_latency_ns(8, 1);
        let _ = latency;
    }
}

