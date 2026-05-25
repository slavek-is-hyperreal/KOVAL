use std::fs;
use schema::{NumaNode, NumaProfile};

pub fn collect() -> NumaProfile {
    collect_internal("/sys/devices/system/node")
}

fn collect_internal(base_path: &str) -> NumaProfile {
    let online_path = format!("{}/online", base_path);
    let online_content = match fs::read_to_string(&online_path) {
        Ok(c) => c,
        Err(_) => return NumaProfile { node_count: 0, nodes: vec![] },
    };

    let node_ids = parse_range_list(&online_content);
    let mut nodes = Vec::new();

    for id in &node_ids {
        let cpulist_path = format!("{}/node{}/cpulist", base_path, id);
        let meminfo_path = format!("{}/node{}/meminfo", base_path, id);
        let distance_path = format!("{}/node{}/distance", base_path, id);

        let cpu_list = fs::read_to_string(&cpulist_path)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let memory_mb = fs::read_to_string(&meminfo_path)
            .map(|s| parse_meminfo_total_mb(&s))
            .unwrap_or(0);

        let distances = fs::read_to_string(&distance_path)
            .map(|s| parse_distances(&s))
            .unwrap_or_default();

        nodes.push(NumaNode {
            id: *id,
            cpu_list,
            memory_mb,
            distances,
        });
    }

    NumaProfile {
        node_count: nodes.len() as u32,
        nodes,
    }
}

fn parse_range_list(content: &str) -> Vec<u32> {
    let mut ids = Vec::new();
    for part in content.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if part.contains('-') {
            let mut bounds = part.split('-');
            if let (Some(start_str), Some(end_str)) = (bounds.next(), bounds.next()) {
                if let (Ok(start), Ok(end)) = (start_str.parse::<u32>(), end_str.parse::<u32>()) {
                    for id in start..=end {
                        ids.push(id);
                    }
                }
            }
        } else {
            if let Ok(id) = part.parse::<u32>() {
                ids.push(id);
            }
        }
    }
    ids
}

fn parse_meminfo_total_mb(content: &str) -> u64 {
    for line in content.lines() {
        if line.contains("MemTotal:") {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if let Some(pos) = parts.iter().position(|&s| s == "MemTotal:") {
                if pos + 1 < parts.len() {
                    if let Ok(kb) = parts[pos + 1].parse::<u64>() {
                        return kb / 1024;
                    }
                }
            }
        }
    }
    0
}

fn parse_distances(content: &str) -> Vec<u32> {
    content
        .split_whitespace()
        .filter_map(|s| s.parse::<u32>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_range_list() {
        assert_eq!(parse_range_list("0"), vec![0]);
        assert_eq!(parse_range_list("0-2"), vec![0, 1, 2]);
        assert_eq!(parse_range_list("0-1,3,5-6"), vec![0, 1, 3, 5, 6]);
        assert_eq!(parse_range_list("abc"), Vec::<u32>::new());
        assert_eq!(parse_range_list(""), Vec::<u32>::new());
    }

    #[test]
    fn test_parse_meminfo_total_mb() {
        let sample = "Node 0 MemTotal:       16345672 kB\nNode 0 MemFree:         435672 kB";
        assert_eq!(parse_meminfo_total_mb(sample), 15962);

        let invalid = "Node 0 MemTotal:       abc kB";
        assert_eq!(parse_meminfo_total_mb(invalid), 0);

        let missing = "Node 0 MemFree:         435672 kB";
        assert_eq!(parse_meminfo_total_mb(missing), 0);
    }

    #[test]
    fn test_parse_distances() {
        assert_eq!(parse_distances("10 20"), vec![10, 20]);
        assert_eq!(parse_distances("10\t20\n30"), vec![10, 20, 30]);
        assert_eq!(parse_distances("abc 10 def"), vec![10]);
        assert_eq!(parse_distances(""), Vec::<u32>::new());
    }

    #[test]
    fn test_collect_internal() {
        let temp_dir = std::env::temp_dir().join("koval_numa_tests");
        let _ = fs::create_dir_all(&temp_dir);

        // Setup online nodes
        fs::write(temp_dir.join("online"), "0-1\n").unwrap();

        // Node 0
        let node0_dir = temp_dir.join("node0");
        fs::create_dir_all(&node0_dir).unwrap();
        fs::write(node0_dir.join("cpulist"), "0-3\n").unwrap();
        fs::write(node0_dir.join("meminfo"), "Node 0 MemTotal:       4194304 kB\n").unwrap();
        fs::write(node0_dir.join("distance"), "10 20\n").unwrap();

        // Node 1
        let node1_dir = temp_dir.join("node1");
        fs::create_dir_all(&node1_dir).unwrap();
        fs::write(node1_dir.join("cpulist"), "4-7\n").unwrap();
        fs::write(node1_dir.join("meminfo"), "Node 1 MemTotal:       8388608 kB\n").unwrap();
        fs::write(node1_dir.join("distance"), "20 10\n").unwrap();

        let profile = collect_internal(&temp_dir.to_string_lossy());
        assert_eq!(profile.node_count, 2);
        assert_eq!(profile.nodes.len(), 2);

        assert_eq!(profile.nodes[0].id, 0);
        assert_eq!(profile.nodes[0].cpu_list, "0-3");
        assert_eq!(profile.nodes[0].memory_mb, 4096);
        assert_eq!(profile.nodes[0].distances, vec![10, 20]);

        assert_eq!(profile.nodes[1].id, 1);
        assert_eq!(profile.nodes[1].cpu_list, "4-7");
        assert_eq!(profile.nodes[1].memory_mb, 8192);
        assert_eq!(profile.nodes[1].distances, vec![20, 10]);

        // Clean up
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
