# Koval Development Manual

**Target Audience:** New kontrybutor entering the project from scratch.

Welcome, developer! This guide provides everything you need to set up, test, debug, and extend Koval. By following these steps, you will establish a local sandbox environment and gain an understanding of how to implement new features inside the orchestrator.

---

## 1. Local Workspace Overview

Koval is structured as a multi-package Cargo workspace. Here are the core development targets:

* **schema** (`schema/src/lib.rs`): Logicless, shared data contract crate.
* **probe** (`probe/src/main.rs`): Lightweight targets diagnostic agent.
* **server** (`server/src/main.rs`): Axum-based HTTP build manager, SQLite manager, and queue processor.
* **cli** (`cli/src/main.rs`): koval CLI — token, webhook, and job management.

---

## 2. Bootstrapping Your Development Environment

### Run Automated Workspace Tests
All workspace unit and integration test blocks compile and execute inside an isolated Docker container, ensuring identical results regardless of host configurations. Run:

```bash
docker compose -f docker-compose.test.yml up --build --abort-on-container-exit
```

If all tests pass, your environment is ready.

### Starting a Local Koval Server Sandbox
To test the Axum orchestrator server locally, configure the proper `KOVAL_*` environment variables and start the server package:

```bash
# Set environment flags and run the server crate
KOVAL_DB="koval.db" \
KOVAL_PORT="8080" \
KOVAL_ARTIFACTS_DIR="artifacts" \
KOVAL_QUEUE_CAPACITY="10" \
KOVAL_RATE_LIMIT="20" \
cargo run -p server
```

Upon boot, the server checks the `tokens` table in SQLite (`koval.db`). If empty, it automatically bootstraps a default developer API token and prints it to standard output:

```text
=======================================================
  BOOTSTRAPPED DEFAULT DEVELOPER ADMIN TOKEN:
  Bearer Token: koval_tkn_default_admin
=======================================================
```

Keep this token handy; you will use it to authorize curl API commands.

---

## 3. Step-by-Step Tutorial: Adding a Hardware Parameter

This tutorial demonstrates how to add a new hardware property — `hyperthreading` detection — to the probe collector and propagate it to the orchestrator.

### Step 1: Add Fields to the Shared Crate
Open `schema/src/lib.rs` and update the `CpuProfile` structure to include the new boolean field:

```rust
// schema/src/lib.rs

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CpuProfile {
    pub flags: Vec<String>,
    pub cache_topology: String,
    pub core_count: usize,
    pub hyperthreading: bool, // <-- Add this field
}
```

Update the `CpuProfile` constructor under the unit test mod (`test_hardware_profile_roundtrip`) to avoid compiler errors.

### Step 2: Implement Hyperthreading Detection in Probe
Open [probe/src/cpu.rs](file:///my_data/KOVAL/probe/src/cpu.rs) and implement the hardware inspection routine (e.g. by comparing CPU core topologies in `/sys` or checking processor capabilities). 

Update the `collect` constructor to parse the hyperthreading property:

```rust
// probe/src/cpu.rs
use schema::CpuProfile;

pub fn collect() -> CpuProfile {
    let flags = read_cpu_flags();
    let cache_topology = parse_cache();
    let core_count = num_cpus::get();
    
    // Simple mock detection: if logical threads are double physical cores
    let hyperthreading = check_hyperthreading_status();

    CpuProfile {
        flags,
        cache_topology,
        core_count,
        hyperthreading, // <-- Assign the new collected field
    }
}
```

### Step 3: Extend Optimization Conditions in Server Forge
Now that the server receives this new hardware parameter inside the JSON `HardwareProfile` payload, you can add a rules compiler matching condition.

Open `server/src/forge.rs` and add the field to `ForgeRule`:

```rust
// server/src/forge.rs

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ForgeRule {
    pub cpu_flags: Option<Vec<String>>,
    pub min_cores: Option<usize>,
    pub hyperthreading_required: Option<bool>, // <-- Add rule match criteria
    
    pub rustflags: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub features: Option<Vec<String>>,
}
```

Then, add the validation check inside `build_config`:

```rust
// Inside build_config(...) in server/src/forge.rs

// 9. Hyperthreading requirement
if let Some(req_ht) = rule.hyperthreading_required {
    if hardware.cpu.hyperthreading != req_ht {
        matched = false;
    }
}
```

Save all files, verify compilation, and run tests locally:
```bash
cargo test --workspace
```
---

## 4. SQLite Schema Verification & Debugging

During development, you may want to inspect databases, check compile states, or add custom test tokens.

Connect to the sqlite database using standard CLI utilities:
```bash
sqlite3 koval.db
```

### Useful SQL Queries

* **View build status of compiled jobs**:
  ```sql
  SELECT id, project, status, queued_at, started_at, finished_at, error_msg FROM jobs;
  ```
  *(Expected status results: `"queued"`, `"building"`, `"done"`, `"failed"`)*.

* **Add a custom active API token**:
  ```sql
  -- Insert a bcrypt-hashed active API token
  -- Hash: "$2b$04$CgYgqKq.3yB6dF23U7PXeugO..." corresponds to raw token "my_dev_key"
  INSERT INTO tokens (token_hash, name, created_at, is_active) 
  VALUES ('$2b$04$CgYgqKq.3yB6dF23U7PXeugO06Y.442lZsc7kY7XyF1.mZkG4/qae', 'My Key', '2026-05-17T17:53:00Z', 1);
  ```

---

## 5. Using the koval CLI

The `koval` command-line tool provides a quick interface to configure access, monitor jobs, manage webhooks, and administer credentials.

### Configuration (One-Time Setup)

By default, the CLI reads and writes its settings in `~/.config/koval/config.json`. Configure the CLI tool with your Koval build server details:

```bash
# Set the server location
koval config set-server http://localhost:8080

# Authenticate using your Bearer token
koval config set-token koval_tkn_default_admin

# Verify the configured settings
koval config show
```

### Supported Subcommand Reference

* **config**: Settings administration.
  * `set-server <url>` — Save the target Koval server base URL.
  * `set-token <token>` — Save the Bearer authentication token.
  * `show` — Display current configuration paths and server settings.
* **token**: Administrative token credentials. *Note: Requires default admin privileges.*
  * `create --name <name>` — Create a new developer Bearer token.
  * `list` — List all registered active tokens.
  * `delete <id>` — Revoke and deactivate a token by ID.
* **job**: Tracking and histories.
  * `list` — List last 50 jobs for the active authenticated token.
  * `status <job_id>` — Inspect the detailed raw JSON status payload for a specific job.
* **webhook**: Integration notifications.
  * `create --url <url> --secret <secret>` — Register a new webhook target endpoint with HMAC signing secret.
  * `list` — List registered webhooks for the active token.
  * `delete <id>` — Revoke and deactivate a webhook endpoint by ID.

## 6. Adding a New Cross-Compilation Target

Koval's supported cross-compilation targets are defined in a single file: `server/src/targets.rs`. Adding a new target takes four steps and touches three files.

Before starting, verify that:
- A GCC cross-linker package for the target exists in the Debian/Ubuntu apt repositories.
- The Rust target triple is available via `rustup target add <triple>`.

### Step 1: Add the target triple to `targets.rs`

Open `server/src/targets.rs` and add the triple to `SUPPORTED_TARGETS`:

```rust
pub const SUPPORTED_TARGETS: &[&str] = &[
    "aarch64-unknown-linux-gnu",
    "armv7-unknown-linux-gnueabihf",
    "x86_64-unknown-linux-musl",
    "your-new-triple-here",   // <-- add here
];
```

Then add a branch in `linker_env_for_target` mapping the triple to its linker binary:

```rust
let linker_bin = match triple {
    "aarch64-unknown-linux-gnu"     => "aarch64-linux-gnu-gcc",
    "armv7-unknown-linux-gnueabihf" => "arm-linux-gnueabihf-gcc",
    "x86_64-unknown-linux-musl"     => "musl-gcc",
    "your-new-triple-here"          => "the-apt-linker-binary",  // <-- add here
    _ => return None,
};
```

The `CARGO_TARGET_<TRIPLE>_LINKER` environment variable name is derived automatically
from the triple — you do not need to hardcode it.

Add unit tests for the new triple in the `tests` module at the bottom of the file:

```rust
#[test]
fn test_is_supported_your_target() {
    assert!(is_supported("your-new-triple-here"));
}

#[test]
fn test_linker_env_your_target() {
    let res = linker_env_for_target("your-new-triple-here").unwrap();
    assert_eq!(res.0, "CARGO_TARGET_YOUR_NEW_TRIPLE_HERE_LINKER");
    assert_eq!(res.1, "the-apt-linker-binary");
}
```

### Step 2: Install the toolchain in `Dockerfile`

Open `Dockerfile` and add the apt package to the runtime stage's install block:

```dockerfile
RUN apt-get update && apt-get install -y \
    ...
    the-apt-package-name \       # <-- add cross-linker package
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add aarch64-unknown-linux-gnu \
    && rustup target add armv7-unknown-linux-gnueabihf \
    && rustup target add x86_64-unknown-linux-musl \
    && rustup target add your-new-triple-here    # <-- add here
```

### Step 3: Mirror the change in `Dockerfile.test`

`Dockerfile.test` must match the production environment exactly.
Add the same apt package and `rustup target add` line to `Dockerfile.test`.

If the toolchain is missing from the test image, cross-compilation integration
tests will pass locally but fail in CI with a linker-not-found error at runtime.

### Step 4: Run the test suite

```bash
docker compose -f docker-compose.test.yml up --build --abort-on-container-exit
```

All existing tests must still pass. The new target appears automatically in
the `400 Bad Request` validation path — no route changes are needed.

---

### Reference: currently supported targets

| Target triple | Architecture | Use case | Linker package |
|---|---|---|---|
| `aarch64-unknown-linux-gnu` | ARM64 | Raspberry Pi 4, AWS Graviton, Apple Silicon servers | `gcc-aarch64-linux-gnu` |
| `armv7-unknown-linux-gnueabihf` | ARM32 (ARMv7) | Raspberry Pi OS 32-bit, embedded ARM | `gcc-arm-linux-gnueabihf` |
| `x86_64-unknown-linux-musl` | x86_64 musl | Static binaries, Alpine containers | `musl-tools` |
| `i686-unknown-linux-musl` | x86 (32-bit) musl | Old Intel CPUs, netbooks, legacy servers | `gcc-multilib` |
| `arm-unknown-linux-gnueabihf` | ARM32 (ARMv6 hard float) | Raspberry Pi 1, Raspberry Pi Zero (ARMv6) | `gcc-arm-linux-gnueabihf` |
| *(none)* | native | Build box architecture | *(no cross-linker needed)* |
