# Koval Development Manual

**Target Audience:** New kontrybutor entering the project from scratch.

Welcome, developer! This guide provides everything you need to set up, test, debug, and extend Koval. By following these steps, you will establish a local sandbox environment and gain an understanding of how to implement new features inside the orchestrator.

---

## 1. Local Workspace Overview

Koval is structured as a multi-package Cargo workspace. Here are the core development targets:

* **[schema](file:///my_data/KOVAL/schema/src/lib.rs)**: Logicless, shared data contract crate.
* **[probe](file:///my_data/KOVAL/probe/src/main.rs)**: Lightweight targets diagnostic agent.
* **[server](file:///my_data/KOVAL/server/src/main.rs)**: Axum-based HTTP build manager, SQLite manager, and queue processor.

---

## 2. Bootstrapping Your Development Environment

### Run Automated Workspace Tests
All workspace unit and integration test blocks compile and execute inside an isolated Docker container, ensuring identical results regardless of host configurations. Run:

```bash
docker compose -f docker-compose.test.yml up --build --abort-on-container-exit
```

If all 9 tests pass, your environment is ready.

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
Open [schema/src/lib.rs](file:///my_data/KOVAL/schema/src/lib.rs) and update the `CpuProfile` structure to include the new boolean field:

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

Open [server/src/forge.rs](file:///my_data/KOVAL/server/src/forge.rs) and add the field to `ForgeRule`:

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
