# Koval Roadmap

This is a living document. Priorities shift as the project grows and as people
actually use it and report what matters. MIT license — everything here is open
for contribution.

---

## What exists today

The core loop works end-to-end:

- **probe** — collects CPU flags, cache topology, memory bandwidth (measured),
  storage stack (io_uring, O_DIRECT, SSD throughput), GPU via Vulkan/ash,
  NUMA topology, CPU base/max frequencies, and kernel version
- **server** — HTTP API, bounded job queue, async worker, SQLite state,
  bcrypt token auth, sliding-window rate limiting
- **webhooks** — HMAC-signed POST delivery on job completion or failure
- **token management** — create/list/revoke via API and CLI
- **job history** — GET /jobs, browser UI at GET /ui
- **koval CLI** — config, token, job, webhook subcommands
- **cross-compilation** — support for compiling to `aarch64-unknown-linux-gnu`, `armv7-unknown-linux-gnueabihf`, and `x86_64-unknown-linux-musl` target architectures
- **workspace support** — multi-binary and workspace auto-detection
- **build cache** — cache hits based on hardware profile and project/git reference
- **koval.toml self-build** — Koval is configured to compile itself with optimized hardware-aware rules
- **production deployment** — ready-to-use Dockerfile, target architecture options, and docker-compose configurations
- **PGO support** — Profile-Guided Optimization (PGO) two-phase compilation pipeline, raw profile uploading (`.profraw`), automated profile merging (`llvm-profdata`), and CLI subcommands `pgo instrument` and `pgo upload`

What it cannot do yet is listed below.

---

## Next — things that should exist before calling this v1

### GitHub Actions integration

A `koval-action` that runs the probe in CI, submits the build to
a self-hosted Koval server, and downloads the result. The probe
would collect the runner's hardware profile, which is deterministic
per runner type — so cached builds would hit reliably.

### Richer probe measurements

Current probe measures memory bandwidth with a simple copy benchmark, NUMA topology, CPU frequencies, and kernel version.
What would make it substantially more useful:

- **Memory latency** — random-access latency matters more than bandwidth
  for pointer-heavy workloads

### Incremental builds

Clone the repo, check if `target/` from a previous build is available
for this `git_ref`, use `--incremental`. Requires artifact storage to
keep the `target/` directory between builds, not just the final binary.
High complexity, high payoff for large projects.

---

## Long term — things that would make this something else entirely

### C and C++ support

Clang compiles to LLVM IR. The koval.toml rule engine is language-agnostic
already — it maps hardware properties to compiler flags. Supporting C/C++
means adding clang invocation alongside cargo in `worker.rs` and extending
`koval.toml` to specify which build system to use.

The Linux kernel compiles with clang. A Koval-built kernel optimized for
the exact machine it runs on is not a ridiculous idea.

### BOLT binary optimization

Facebook's BOLT post-link optimizer rewrites binary layout based on
execution profiles. Same two-pass structure as PGO, applied after
linking. For latency-sensitive binaries, BOLT improvements can be
substantial on top of PGO.

### GPU-accelerated LLVM passes

This is speculative. The detailed hypothesis and research protocol for this concept are parked in [ideas/gpu_compilation_research.md](file:///my_data/KOVAL/ideas/gpu_compilation_research.md).

### Smart installer use case

A small static binary (musl) that runs the probe, posts to a vendor's
self-hosted Koval server, and installs the result. The vendor ships
`curl -s https://get.myapp.com | sh` — the user gets a binary compiled
for their exact hardware without installing Rust or any toolchain. The
build cache makes this economically viable: the first user with a given
hardware profile triggers compilation, all subsequent users with the
same profile get a cached binary instantly.

### Hardware-bound licensing

Vendor-side license key validation tied to the target machine hardware
fingerprint. A license key issued at purchase is bound to the probe's
hardware profile on first activation. Subsequent builds verify the key
matches the original fingerprint. Enables Koval as a distribution and
activation layer for commercial software.

---

## What this will never try to be

A general-purpose CI system. There are excellent options for that.
Koval does one thing: it knows the hardware and forges binaries for it.

---

## Contributing

All of the above is open. If something on this list matters to you,
open an issue before starting — not to ask permission, but to avoid
duplicating work and to get early feedback on the approach.

The things most likely to be useful to more people, roughly in order:
GitHub Actions integration, memory latency probe, incremental builds.

The things most likely to be interesting to work on, in no particular
order: BOLT, PGO, smart installer, hardware-bound licensing.