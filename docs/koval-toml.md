# koval.toml Reference Guide

**Target Audience:** Developer adding Koval support to their Rust project.

Welcome! This guide explains how to add Koval optimization rules to your Rust project. By placing a `koval.toml` file in your repository, you allow Koval's compilation coordinator to automatically match client hardware profiles against specific compilation criteria and apply customized compiler configurations.

---

## 1. File Placement

The `koval.toml` file must be placed in the **root directory** of your cargo repository, directly next to your project's `Cargo.toml` file.

```text
my_computation_engine/
├── Cargo.toml
├── build.rs
├── koval.toml          <-- Add this file here!
└── src/
    └── main.rs
```

---

## 2. Configuration Schema

A `koval.toml` file is composed of an array of tables named `[[rules]]`. Each rule acts as a conditional matching layer: if the target machine meets all the hardware requirements specified in a rule, the rule's associated compiler actions (`rustflags`, `env`, and `features`) are collected and applied.

### Hardware Match Criteria (Conditional Fields)
All specified criteria in a single rule must be met for the rule to apply (logical AND).

| Field Name | Type | Description |
|---|---|---|
| `cpu_flags` | Array of Strings | Required CPU instruction sets (e.g. `["avx2", "aes"]`). |
| `min_cores` | Integer | Minimum logical CPU cores required. |
| `min_memory_gb` | Float | Minimum target physical memory capacity in Gigabytes. |
| `min_memory_bandwidth` | Float | Minimum target RAM memory bandwidth in MB/s. |
| `require_io_uring` | Boolean | Whether target kernel must support `io_uring`. |
| `require_o_direct` | Boolean | Whether target file system must support aligned `O_DIRECT`. |
| `min_storage_read_mbs` | Float | Minimum physical storage read throughput in MB/s. |
| `min_gpu_vram_gb` | Float | Minimum dedicated VRAM capacity of at least one target GPU in Gigabytes. |

### Compilation Actions (Output Fields)
If all conditions in the rule match, these actions are collected.

| Field Name | Type | Description |
|---|---|---|
| `rustflags` | Array of Strings | Compiler flags passed as space-separated tokens in the `RUSTFLAGS` environment variable. |
| `env` | Table (Key-Value) | Custom system environment variables injected into the compiler execution shell. |
| `features` | Array of Strings | Cargo feature flags enabled during compilation (passed to `--features`). |

---

## 3. Real `koval.toml` Example

Below is a complete, production-ready `koval.toml` demonstrating vector optimization flags, memory alignment configurations, and GPU features:

```toml
# koval.toml

# Rule 1: Enable AVX2 vector optimizations if supported by the CPU
[[rules]]
cpu_flags = ["avx2", "sse4.1"]
rustflags = ["-C", "target-feature=+avx2"]
env = { MY_APP_VECTORIZED = "true" }
features = ["avx2-acceleration"]

# Rule 2: Configure high performance caching rules if target RAM bandwidth is high
[[rules]]
min_cores = 8
min_memory_gb = 15.5
min_memory_bandwidth = 20000.0
env = { MY_APP_CACHE_SIZE = "4194304" } # 4MB cache size

# Rule 3: Enable Direct IO drivers if file system and io_uring support it
[[rules]]
require_io_uring = true
require_o_direct = true
min_storage_read_mbs = 400.0
features = ["direct-io-driver"]

# Rule 4: Enable GPU acceleration if matching graphics card VRAM exceeds 8 GB
[[rules]]
min_gpu_vram_gb = 8.0
rustflags = ["-C", "opt-level=3"]
features = ["gpu-accelerated"]
```

---

## 4. The `build.rs` and Cargo Integration

When Koval builds your project, it aggregates all matching features, passes them as `--features` to your compiler, and injects matched `env` keys as compile-time environment variables. 

### build.rs Integration Pattern
Your `build.rs` script can dynamically inspect these injected environment parameters and write configuration definitions for your Rust source files. You must implement safe fallbacks for standard developer environments.

```rust
// build.rs
use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn main() {
    // 1. Read environment parameters injected by koval.toml matched rules
    let vectorized = env::var("MY_APP_VECTORIZED")
        .unwrap_or_else(|_| "false".to_string());
    let cache_size = env::var("MY_APP_CACHE_SIZE")
        .unwrap_or_else(|_| "1048576".to_string()); // Default: 1MB

    // 2. Generate configuration header file
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("koval_config.rs");
    let mut f = File::create(&dest_path).unwrap();

    writeln!(
        f,
        "pub const APP_VECTORIZED: bool = {};\n\
         pub const APP_CACHE_SIZE: usize = {};",
        vectorized, cache_size
    ).unwrap();

    // Re-run build script if koval.toml changes
    println!("cargo:rerun-if-changed=koval.toml");
}
```

### Cargo.toml Feature Declarations
All features listed in the `features` arrays inside your `koval.toml` rules must be explicitly declared in your project's `Cargo.toml`:

```toml
# Cargo.toml

[package]
name = "my_computation_engine"
version = "0.1.0"
edition = "2021"

[features]
default = []
avx2-acceleration = []
direct-io-driver = []
gpu-accelerated = []
```

---

## 5. Local Verification

To verify that your rules and configuration map correctly, run these two local compiler checks:

```bash
# 1. Standard build verification (simulating standard development compilation)
cargo check --all-targets

# 2. Optimized simulation (manually providing environment variables and matching features)
MY_APP_VECTORIZED=true \
MY_APP_CACHE_SIZE=4194304 \
cargo check --features "avx2-acceleration direct-io-driver"
```
