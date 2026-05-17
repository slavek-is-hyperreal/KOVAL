# Contributing to Koval

**Target Audience:** Developer who knows Rust, wants to contribute.

Welcome! We are excited that you want to contribute to Koval. This brief guide outlines the process, formatting rules, and guidelines for submitting pull requests.

---

## 1. What Contributions are Welcome?

We welcome a wide range of contributions, including:
- **Bug Fixes**: Rectifying issues in the orchestrator server, sqlite integrations, or dynamic Vulkan linkages.
- **Probe Collectors**: Expanding the target diagnostics agent to capture new hardware or kernel characteristics.
- **Forge Rules**: Designing new optimization conditions inside `forge.rs` to support more complex target matching.
- **Documentation**: Improving guides, schemas, or adding deployment blueprints.

To discuss new feature concepts or significant architecture changes before writing code, please open an Issue on our repository.

---

## 2. Prerequisites

To contribute effectively, your local development system must have:
- **Rust Toolchain**: Stable version 1.70 or newer.
- **Docker & Docker Compose**: Used to compile and run tests in isolated sandbox environments.
- **Git**: Used for tracking code updates and branch management.

---

## 3. Running the Test Suite

Every pull request must pass our continuous integration suite. Before committing, verify all packages by running our isolated testing container:

```bash
docker compose -f docker-compose.test.yml up --build --abort-on-container-exit
```

---

## 4. Branch Conventions & PRs

- **Branch Naming**: Keep branch names structured and descriptive:
  - Features: `feature/add-temperature-collector`
  - Bug Fixes: `bugfix/fix-direct-io-buffer`
  - Documentation: `docs/update-api-examples`
- **PR Description**: Include a clear description of the problem solved, list the modified files, and confirm that the Docker test suite succeeded.

---

## 5. Coding Standards & Style

To maintain code health across all Koval packages, ensure your changes follow these rules:

- **Formatting**: Run `cargo fmt` to format all workspace files before committing.
- **Clippy Checks**: Run `cargo clippy --all-targets` to catch optimization or safety issues. Your code must compile with zero compiler warnings.
- **Failsafe Operations**: **Strictly no calls to `.unwrap()` or `.expect()`** inside non-test libraries or server runtime code. All functions must propagate errors gracefully via standard `Result<T, E>` types to ensure the probe and orchestrator server never panic or crash under unexpected environments.
