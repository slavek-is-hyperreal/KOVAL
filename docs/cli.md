# Koval CLI Terminal Reference

**Target Audience:** Developer using koval to manage their build server from the terminal.

Welcome to the command-line interface (CLI) manual for **Koval**! The `koval` terminal utility provides a lightweight, highly efficient interface to configure server connection details, inspect build histories, register webhook destinations, and administer Bearer tokens.

---

## 1. Installation

The CLI utility is built as a native binary within the multi-crate Koval workspace. To compile the application and locate the binary, execute the release build command from the repository root:

```bash
# Compile the koval CLI utility in release mode
cargo build -p koval-cli --release

# The compiled binary is generated at:
# target/release/koval
```

For convenience, you can symlink or move the binary into your standard system execution path:
```bash
sudo cp target/release/koval /usr/local/bin/
```

---

## 2. Configuration Settings

The CLI tool stores its settings in a local JSON configuration file located at:
`~/.config/koval/config.json`

Before calling any status or administration operations, you must perform a one-time configuration setup to define the base URL of your Koval server and save your developer Bearer token.

### A. Set Koval Server Base URL
```bash
koval config set-server http://localhost:8080
```

### B. Save Developer Bearer Token
```bash
koval config set-token koval_tkn_default_admin
```

### C. Display Configuration Settings
```bash
koval config show
```
*Expected Output Format:*
```text
Config file: Some("/home/developer/.config/koval/config.json")
Server URL:  http://localhost:8080
Token:       ********
```

---

## 3. Token Administration

Manage developer Bearer tokens authorized to submit build jobs and manage webhook subscriptions.

> [!IMPORTANT]
> **Admin Privileges Limitation:**
> In accordance with personal-use environment parameters, token management commands strictly require your authenticated Bearer token in the configuration settings to correspond directly to the bootstrapped default admin token (`koval_tkn_default_admin`). Subordinate tokens created via these commands do not possess privileges to query or modify token registries.

### A. Create a New Bearer Token
```bash
koval token create --name "staging-server-1"
```
*Expected Output:*
```text
=======================================================
  TOKEN CREATED SUCCESSFULLY
  Name:      staging-server-1
  ID:        2
  Token:     9ab1c3d5-e4f6-47b8-89c0-11d2e3f4a5b6
=======================================================
  WARNING: Copy this token immediately. It will NOT
  be shown again and cannot be retrieved!
=======================================================
```

### B. List Registered Active Tokens
```bash
koval token list
```
*Expected Output:*
```text
+----+--------------------+----------------------+
| ID | Name               | Created At           |
+----+--------------------+----------------------+
| 1  | admin              | 2026-05-17T18:00:00Z |
| 2  | staging-server-1   | 2026-05-17T18:30:15Z |
+----+--------------------+----------------------+
```

### C. Revoke / Deactivate a Token by ID
```bash
koval token delete 2
```
*Expected Output:*
```text
Token successfully revoked/deactivated.
```

---

## 4. Job History & Status Monitoring

Track, monitor, and query compilation jobs associated with the currently authenticated Bearer token.

### A. List Recent Compilation Jobs
```bash
koval job list
```
*Expected Output:*
```text
+--------------------------------------+---------------------------------+---------+----------------------+----------------------+
| Job ID                               | Project                         | Status  | Started At           | Finished At          |
+--------------------------------------+---------------------------------+---------+----------------------+----------------------+
| 7f18b456-c392-4911-897b-928efad984d8 | https://github.com/org/proj.git | done    | 2026-05-17T18:05:00Z | 2026-05-17T18:07:30Z |
| 1a8c6b3d-4e5f-6a7b-8c9d-0e1f2a3b4c5d | https://github.com/org/proj.git | failed  | 2026-05-17T18:10:00Z | 2026-05-17T18:11:15Z |
+--------------------------------------+---------------------------------+---------+----------------------+----------------------+
```

### B. Inspect Detailed Job Status JSON
```bash
koval job status 7f18b456-c392-4911-897b-928efad984d8
```
*Expected Output JSON:*
```json
{
  "status": "done",
  "queued_at": "2026-05-17T18:04:55Z",
  "started_at": "2026-05-17T18:05:00Z",
  "finished_at": "2026-05-17T18:07:30Z",
  "error_msg": null,
  "position": null
}
```

---

## 5. Webhook Subscriptions

Manage dynamic HTTP webhook targets notified automatically when build jobs complete.

### A. Register a Webhook Target
```bash
koval webhook create --url "https://ci.example.com/hooks/koval" --secret "my_signing_secret_123"
```
*Expected Output:*
```text
Webhook registered successfully.
```

### B. List Registered Webhook Channels
```bash
koval webhook list
```
*Expected Output:*
```text
+----+-------------------------------------+----------------------+--------+
| ID | URL                                 | Created At           | Active |
+----+-------------------------------------+----------------------+--------+
| 1  | https://ci.example.com/hooks/koval  | 2026-05-17T18:15:00Z | true   |
+----+-------------------------------------+----------------------+--------+
```

### C. Delete / Deactivate a Webhook Channel
```bash
koval webhook delete 1
```
*Expected Output:*
```text
Webhook successfully deleted/deactivated.
```

---

## 6. Profile-Guided Optimization (PGO)

Manage Profile-Guided Optimization two-phase pipelines from the terminal.

### A. Submit Instrumentation Job
```bash
koval pgo instrument "https://github.com/org/proj.git" --git-ref "main" --cpu "native"
```
*Expected Output:*
```text
=======================================================
  PGO INSTRUMENTATION JOB SUBMITTED SUCCESSFULLY
  Job ID: 7f18b456-c392-4911-897b-928efad984d8
=======================================================
```

### B. Upload Profiles & Trigger Optimization
Once the instrumented binary has been executed on the target environment and has produced raw profiling files (`*.profraw`), place them in a directory and run the upload command:
```bash
koval pgo upload 7f18b456-c392-4911-897b-928efad984d8 ./profiles_dir
```
*Expected Output:*
```text
Uploading 3 .profraw files...
=======================================================
  PGO PROFILES UPLOADED & MERGED SUCCESSFULLY
  Merged Profile URL:   /pgo/profiles/7f18b456-c392-4911-897b-928efad984d8/merged.profdata
  Optimization Job ID:  77e38202-b2d9-4809-9134-8c8a74b48cc1
=======================================================
```

---

## 7. Complete Command Reference Table

The table below catalogs all CLI execution patterns supported by the `koval` binary:

| Subcommand | Option / Args | Function Description |
|---|---|---|
| `config set-server` | `<url>` | Sets and saves the base address of the build orchestrator server. |
| `config set-token` | `<token>` | Sets and saves the Bearer authentication token. |
| `config show` | *(none)* | Displays the config file path, current server URL, and masked token state. |
| `token create` | `--name <name>` | Creates a new user token and outputs its plaintext Bearer string. |
| `token list` | *(none)* | Prints a tabular list of all active tokens. |
| `token delete` | `<id>` | Revokes access for the specified token ID. |
| `job list` | *(none)* | Displays a history grid of the last 50 build jobs. |
| `job status` | `<job_id>` | Fetches and formats the complete raw status JSON of a specific build job. |
| `webhook create` | `--url <url> --secret <secret>` | Registers a webhook receiver URL with a signing secret key. |
| `webhook list` | *(none)* | Prints a tabular list of all webhook subscriptions. |
| `webhook delete` | `<id>` | Disables and revokes notifications for the webhook ID. |
| `pgo instrument` | `<project> [--git-ref <ref>] [--cpu <cpu>] [--target <triple>]` | Submits a PGO instrumentation build job. |
| `pgo upload` | `<instrument_job_id> <profiles_dir>` | Uploads raw profiling files, merges them, and triggers PGO optimization. |
