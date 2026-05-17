# Koval

> *Kowal* (Polish) — a blacksmith. One who takes raw metal and forges it into something precise, purpose-built, and exactly right for the hand that will use it.

**Koval is a hardware-aware compilation service.** It profiles the target machine, then forges a binary optimized for that exact CPU microarchitecture, cache topology, memory bandwidth, GPU capabilities, and storage stack — not for some generic baseline.

---

## The Problem

Rust gives you `target-cpu=native` — but only if you compile *on* the target machine. The moment you want a dedicated build box, you lose that. You're back to lowest-common-denominator binaries that leave performance on the table.

For projects where this matters — SIMD-heavy numerical code, custom tensor engines, anything that dispatches differently on AVX vs AVX2 vs SSE2, anything that sizes ring buffers to L2/L3 cache — the difference between a generic binary and a hardware-tuned one isn't 5%. It can be 10x.

Koval solves this by separating **hardware knowledge** (collected on the target) from **compilation** (done on your build box).

---

## How It Works

```
[Target Device]                    [Build Box — Docker]
                                   ┌─────────────────────────────┐
koval-probe                        │  POST /build                │
  → collects:                      │    ← hardware.json + token  │
    · CPU flags (AVX, AVX2,        │    → job_id                 │
      SSE2, F16C, FMA,             │                             │
      NEON, SVE...)                │  GET /build/{id}/status     │
    · Cache topology               │    ← token                  │
      (L1/L2/L3 sizes,             │    → queued/building/done   │
      cache line size)             │                             │
    · Memory bandwidth             │  GET /build/{id}/binary     │
      (measured, not theoretical)  │    ← token                  │
    · Storage stack                │    → optimized binary       │
      (io_uring support,           └─────────────────────────────┘
       O_DIRECT, SSD bandwidth)
    · GPU (Vulkan device props,
       VRAM, PCIe link)

  → POST to Koval server
  → poll status
  → download binary
```

One command on the target. One binary back. Done.

---

## Architecture

Koval is a single Docker container you run on your build machine.

```
┌─────────────────────────────────────────────┐
│  axum HTTP API                              │
│                                             │
│  · Token auth (bcrypt, SQLite-backed)       │
│  · Rate limiting per token (sliding window) │
│  · Job submission and status polling        │
│  · Binary artifact download                 │
└──────────────┬──────────────────────────────┘
               │
┌──────────────▼──────────────────────────────┐
│  Job Queue (tokio bounded channel)          │
│                                             │
│  · Ordered FIFO                             │
│  · Configurable concurrency limit           │
│  · Backpressure on full queue (503)         │
└──────────────┬──────────────────────────────┘
               │
┌──────────────▼──────────────────────────────┐
│  Worker Pool                                │
│                                             │
│  For each job:                              │
│  1. Read project's koval.toml               │
│  2. Map hardware profile → RUSTFLAGS + env  │
│  3. cargo build --release                   │
│  4. Store artifact + SHA256                 │
│  5. Update job status                       │
└──────────────┬──────────────────────────────┘
               │
┌──────────────▼──────────────────────────────┐
│  SQLite                                     │
│                                             │
│  tokens      → auth, active/revoked         │
│  jobs        → queue, status, timestamps    │
│  artifacts   → path, size, checksum         │
│  rate_limit  → per-token sliding window     │
└─────────────────────────────────────────────┘
```

---

## Project Integration

Any Rust project that wants Koval support adds one file: `koval.toml`.

This file declares which hardware conditions activate which compiler features and build-time constants. Koval reads it from the project repository at build time — no changes to the server are needed when you add a new project.

```toml
# koval.toml
[[rules]]
cpu_flags = ["avx2"]
features  = ["avx2"]
rustflags = ["-C", "target-feature=+avx2"]

[[rules]]
require_io_uring = true
features = ["io_uring"]

[[rules]]
min_gpu_vram_gb = 4.0
features = ["vulkan"]
```

→ Full reference: [docs/koval-toml.md](docs/koval-toml.md)

---

## API

### Submit a build job

```
POST /build
Authorization: Bearer <token>
Content-Type: application/json

{
  "project": "https://github.com/you/my-project",
  "git_ref": "main",
  "hardware": { ...probe output... }
}
```

```json
{ "id": "7f18b456-c392-4911-897b-928efad984d8" }
```

### Poll job status

```
GET /build/7f18b456-c392-4911-897b-928efad984d8/status
Authorization: Bearer <token>
```

```json
{
  "status":      "building",
  "queued_at":   "2026-05-17T10:00:00Z",
  "started_at":  "2026-05-17T10:00:05Z",
  "finished_at": null,
  "error_msg":   null,
  "position":    null
}
```

Status values: `queued` → `building` → `done` | `failed`

Rate limited per token (sliding window, configurable). Exceeding returns `429 Too Many Requests`.

### Download binary

```
GET /build/7f18b456-c392-4911-897b-928efad984d8/binary
Authorization: Bearer <token>
```

Returns the compiled binary as a `.tar.gz`. SHA256 checksum is in the `x-sha256` response header. Only available when status is `done`.

---

## Deployment

```bash
docker compose up -d
```

```yaml
# docker-compose.yml
services:
  koval:
    image: koval-server
    ports:
      - "127.0.0.1:8731:8731"
    volumes:
      - ./data/db:/data/db
      - ./data/artifacts:/data/artifacts
      - ./data/repos:/data/repos
    environment:
      - KOVAL_QUEUE_CAPACITY=2
      - KOVAL_RATE_LIMIT=10
      - KOVAL_PORT=8731
      - KOVAL_DB=/data/db/koval.db
      - KOVAL_ARTIFACTS_DIR=/data/artifacts
```

The Docker image ships with the Rust toolchain, all registered `rustup` targets, and any native dependencies your projects need.

---

## Token Management

On first start, Koval prints a default admin token. From there:

```bash
# A token for a new device — shown once, store it
koval-cli token add --name "old-laptop"

# See what's active
koval-cli token list

# Revoke when a device is retired
koval-cli token revoke --name "old-laptop"
```

Tokens are stored as bcrypt hashes. Plaintext is shown once at creation and never again.

---

## Workspace

```
koval/
├── probe/          ← runs on the target device, collects hardware profile
├── schema/         ← shared types between probe and server (no logic)
└── server/         ← HTTP API, queue, worker, SQLite
```

---

## Documentation

Not sure where to start? Pick the one that matches what you're trying to do:

- **Running the probe on a target device** → [docs/probe.md](docs/probe.md)
- **Adding Koval support to your Rust project** → [docs/koval-toml.md](docs/koval-toml.md)
- **Calling the HTTP API from a script** → [docs/api.md](docs/api.md)
- **Understanding how Koval works internally** → [docs/architecture.md](docs/architecture.md)
- **Setting up a local dev environment** → [docs/development.md](docs/development.md)
- **Contributing code** → [CONTRIBUTING.md](CONTRIBUTING.md)

---

## Status

Early development. Probe and server API are the first milestones.

**Not yet implemented:**
- Build cache (same hardware profile + git ref → reuse existing artifact)
- Webhook notifications on job completion
- `koval-cli` token management tool
- Web UI for job history

---

## Name

*Kowal* is the Polish word for blacksmith — someone who doesn't produce generic parts off a shelf, but shapes metal precisely for its intended use. The name reflects the project's philosophy: a binary should be shaped for the machine that will run it, not the machine that compiled it.

---

## License

MIT