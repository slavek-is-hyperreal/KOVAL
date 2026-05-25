mod cpu;
mod memory;
mod storage;
mod gpu;
mod numa;

use schema::HardwareProfile;

fn main() {
    let cpu = cpu::collect();
    let memory = memory::collect();
    let storage = storage::collect();
    let gpu = gpu::collect();
    let numa = numa::collect();

    let profile = HardwareProfile {
        cpu,
        memory,
        storage,
        gpu,
        numa,
    };

    match serde_json::to_string_pretty(&profile) {
        Ok(json) => println!("{}", json),
        Err(e) => {
            eprintln!("Failed to serialize HardwareProfile to JSON: {}", e);
            std::process::exit(1);
        }
    }
}
