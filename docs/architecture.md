# Koval Internal Architecture

**Target Audience:** Developer curious how Koval works internally.

Welcome to Koval's internal design manual! If you are interested in extending Koval, fixing a backend bug, or simply curious about how we orchestrate builds based on target hardware profiles, this document is for you.

---

## Workspace Structure

Koval is organized as a Cargo workspace containing three distinct, highly focused packages:

- **schema** (library): A logicless, pure data definition crate. It contains the shared structures for hardware profiles, job requests, and serialization/deserialization schemas. By keeping it logic-free, we ensure both the probe (which runs on lightweight target devices) and the server (which runs on a powerful build host) share a compiled, rigid API contract without pulling in unnecessary runtime dependencies.
- **probe** (binary): A lightweight system analysis tool designed to run directly on target client devices. Its sole responsibility is to inspect the local machine (CPU flags, RAM capacity, physical disk write/read throughput via `O_DIRECT`, and Vulkan graphics subsystems via dynamic `ash` bindings) and print a clean JSON hardware profile to standard output before exiting.
- **server** (binary): A robust web server and asynchronous build engine built with Axum. It handles token authentication, manages build job records in a local SQLite database, operates a backpressured memory queue, matches incoming client hardware characteristics to compilation configurations, and produces optimized target binaries.

---

## System Data Flow

The diagram below shows how a build request flows from a target device running the probe, through the Koval server queue, and into the local compiler workspace to yield a hardware-optimized binary.

```text
+-------------------+                 +----------------------+
|   Target Device   |                 |  Koval Server Crate  |
+-------------------+                 +----------------------+
          |                                       |
  1. run `probe` binary                           |
          |                                       |
  2. serialized profile                           |
          | --- POST /build (with profile) -----> | [Axum Routing]
          |                                       |       |
          |                                       |   3. Verify Auth Token (bcrypt verification)
          |                                       |   4. Apply sliding rate limits
          |                                       |   5. Compute cache key and check build cache
          |                                       |       | [Cache Hit & File Exists]
          |                                       |       +--> Skip build, immediately return 202 with cached job ID
          |                                       |       | [Cache Miss / File Missing]
          |                                       |   6. Save Job as "queued" in SQLite
          |                                       |   7. Push to Bounded Memory Queue
          |                                       |       |
          |                                       | [Async Worker Loop]
          |                                       |   8. Pop job from queue
          |                                       |   9. Match koval.toml rules (forge.rs)
          |                                       |  10. Run `cargo build` with targeted RUSTFLAGS & features
          |                                       |  11. Save cache record & update status to "done"
          |                                       |       |
          | <--- GET /build/status -------------- | [Status Poll]
          |                                       |
          | <--- GET /build/binary -------------- | [Download packaged tar.gz]
```

---

## Database Architecture

Koval uses an embedded SQLite database (configured by `KOVAL_DB`) to manage state persistence. The server relies on four primary tables:

1. **tokens**: Stores API Bearer tokens allowed to trigger compilation jobs. Instead of storing plain text tokens or fast hashes, the database stores standard salt-backed **bcrypt hashes**. It records the creation date, status (active/revoked), and ownership.
2. **jobs**: Stores the complete record of every compilation request. This table stores the target project name, Git reference, the complete serialized target `HardwareProfile` JSON payload, current status (`queued`, `building`, `done`, `failed`), logs, and file system paths to the completed build archive.
3. **webhooks**: Stores webhook receiver URLs and HMAC secrets mapped to active client tokens, alongside enabled/disabled state flags, used for secure notifications.
4. **build_cache**: Maps unique build cache keys (deterministic hashes of hardware profile, git repo, git ref, and target binary name) to their completed `job_id` and timestamp. Used to bypass compilation on exact build duplicate hits.

---

## Job Queue & Async Worker

To ensure that heavy compilation jobs do not block web requests or starve server resources, Koval uses an asynchronous producer-consumer queue:

- **Enqueueing**: When a valid `POST /build` is received, the job is registered in SQLite as `queued` and pushed into a thread-safe, bounded memory channel. If the queue is full, immediate backpressure is applied, and the client receives a `503 Service Unavailable` response.
- **Worker Thread**: A dedicated, long-running worker loop pops jobs off the queue one by one. Upon popping a job, the worker updates its database status to `building`, clones and checks out the project branch, parses and evaluates `koval.toml` rules against the hardware profile, detects the project compile layout (Workspace, Specific Binary, or Single Package), triggers the targeted `cargo build` compilation with custom features and environment injections, dynamically scans and gathers the output binaries (validating unix executable permission bits for workspaces), packs the compiled target binaries into a `tar.gz` archive, calculates its SHA-256 signature, and transitions the database status to `done` (or `failed` upon error). After transitioning status to `done` or `failed`, the worker reads active webhooks for the job's token from the database and triggers asynchronous HTTP delivery to each registered URL.

---

## Optimization Mapping (forge.rs)

The `forge` module is the brain of Koval's optimization engine. When a build is triggered, the worker parses the target project's `koval.toml` configuration and evaluates the matching rules:

- **Hardware Extraction**: It pulls CPU profiles, available RAM, and GPU presence from the job's hardware profile.
- **Evaluation**: For each rule declared under the `[[rules]]` array in `koval.toml`, the forge compares the target characteristics against specified criteria (e.g. minimum CPU cores, memory bandwidth, or explicit CPU instruction sets like `avx2`).
- **Optimization Compilation**: If all specifications of a rule match, its specified compiler environments (`rustflags`, `env`, and `features`) are collected. The server merges the features, sets target environment variables, and assigns `RUSTFLAGS` before launching `cargo build`.

---

## Authentication & Rate Limiting

- **Bcrypt Token Protection**: All incoming client requests must present a bearer token in the `Authorization` header. The server verifies this token against stored active token hashes using **bcrypt verification** (`bcrypt::verify`). This protects credentials with a computationally heavy validation pipeline, preventing database compromise leakage.
- **Sliding Window Rate Limiter**: To prevent denial-of-service attacks, every API token has an associated sliding-window rate limit stored in SQLite. The system tracks timestamps of requests within a sliding 60-second window. If a client exceeds the maximum allowed count, the server responds immediately with a `429 Too Many Requests` status.
