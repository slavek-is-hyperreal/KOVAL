# Koval HTTP API Reference

**Target Audience:** Developer integrating with the HTTP API.

Welcome to the Koval HTTP API reference! This document provides all the information you need to authenticate, trigger hardware-aware builds, track compiler progress, and download completed artifacts.

---

## Global API Rules

### Authentication
Every request to the Koval API requires an `Authorization` header containing a valid bearer token. If the token is invalid, missing, or revoked, the server returns `401 Unauthorized`.

```http
Authorization: Bearer koval_tkn_default_admin
```

### Rate Limiting
To ensure fair resource sharing, the API enforces a sliding-window rate limit (typically 20 requests per 60 seconds per token, customizable by `KOVAL_RATE_LIMIT`). 
- **Exceeding the limit**: If you exceed this threshold, the server immediately returns a `429 Too Many Requests` status.

---

## Endpoint Directory

### 1. Trigger Hardware-Aware Build
`POST /build`

Triggers a new background compilation job using a target hardware profile supplied in the request body.

#### Request Headers
- `Authorization: Bearer koval_tkn_default_admin` (Required)
- `Content-Type: application/json` (Required)

#### Request Body Schema (JSON)
- **hardware** (Object, Required): The exact hardware profile of the target device.
  - **cpu** (Object):
    - **flags** (Array of Strings): Active CPU instruction sets (e.g. `["avx2", "sse4.1"]`).
    - **cache_topology** (String): CPU cache sizes (e.g. `"L1:32KB,L2:256KB,L3:8MB"`).
    - **core_count** (Integer): Number of logical cores.
  - **memory** (Object):
    - **total_bytes** (Integer): Total physical RAM on the device in bytes.
    - **available_bytes** (Integer): Free physical RAM on the device in bytes.
    - **bandwidth_mbs** (Float): Measured memory bandwidth in MB/s.
  - **storage** (Object):
    - **io_uring** (Boolean): Whether the target kernel supports `io_uring`.
    - **o_direct** (Boolean): Whether the target file system supports aligned `O_DIRECT`.
    - **read_speed_mbs** (Float): Measured read throughput in MB/s.
    - **write_speed_mbs** (Float): Measured write throughput in MB/s.
  - **gpu** (Object):
    - **devices** (Array of Objects): Enumerated Vulkan/DRI GPU profiles. Each object has:
      - **name** (String): Graphics card model name.
      - **vram_bytes** (Integer): Device-local memory capacity in bytes.
      - **pcie_info** (String or Null): PCIe generation and lane count.
- **project** (String, Required): URL or path of the target Rust project to build (e.g. `"https://github.com/example/project.git"`).
- **git_ref** (String, Required): Branch, tag, or exact commit hash to check out (e.g. `"main"`).

#### Response Schema
- **202 Accepted**: The job is successfully authenticated, saved to SQLite, and pushed into the build queue.
  - Body: JSON containing a unique job identifier.
    ```json
    {
      "id": "7f18b456-c392-4911-897b-928efad984d8"
    }
    ```
- **401 Unauthorized**: Missing or invalid authentication token.
- **429 Too Many Requests**: Rate limit exceeded.
- **503 Service Unavailable**: Bounded memory queue is full; request dropped.

#### Example Command
```bash
curl -X POST http://localhost:8080/build \
  -H "Authorization: Bearer koval_tkn_default_admin" \
  -H "Content-Type: application/json" \
  -d '{
    "hardware": {
      "cpu": {
        "flags": ["avx2"],
        "cache_topology": "L1:32KB",
        "core_count": 4
      },
      "memory": {
        "total_bytes": 17179869184,
        "available_bytes": 8589934592,
        "bandwidth_mbs": 24000.0
      },
      "storage": {
        "io_uring": true,
        "o_direct": true,
        "read_speed_mbs": 520.0,
        "write_speed_mbs": 480.0
      },
      "gpu": {
        "devices": [
          {
            "name": "NVIDIA GeForce RTX 4070",
            "vram_bytes": 12884901888,
            "pcie_info": "PCIe Link: x16 @ 16.0 GT/s"
          }
        ]
      }
    },
    "project": "https://github.com/example/project.git",
    "git_ref": "main"
  }'
```

---

### 2. Query Build Job Status
`GET /build/{id}/status`

Retrieves the current execution state, timing, and compilation outcome for a specific build job.

#### Request Headers
- `Authorization: Bearer koval_tkn_default_admin` (Required)

#### Path Parameters
- **id** (String, Required): The unique UUID returned by the build initiation endpoint (e.g. `7f18b456-c392-4911-897b-928efad984d8`).

#### Response Schema
- **200 OK**: Job found. Returns status details.
  - **status** (String): Current state (one of `"queued"`, `"building"`, `"done"`, `"failed"`).
  - **queued_at** (String): ISO 8601 timestamp representing job creation time.
  - **started_at** (String or Null): ISO 8601 timestamp representing when compilation began.
  - **finished_at** (String or Null): ISO 8601 timestamp representing job completion time.
  - **error_msg** (String or Null): Compilation errors or runtime failure diagnostic details.
  - **position** (Integer or Null): Queue position index if state is `"queued"` (1-indexed).
  ```json
  {
    "status": "done",
    "queued_at": "2026-05-17T17:29:45Z",
    "started_at": "2026-05-17T17:29:48Z",
    "finished_at": "2026-05-17T17:30:00Z",
    "error_msg": null,
    "position": null
  }
  ```
- **401 Unauthorized**: Missing or invalid authentication token.
- **404 Not Found**: No job matches the provided identifier.

#### Example Command
```bash
curl -X GET http://localhost:8080/build/7f18b456-c392-4911-897b-928efad984d8/status \
  -H "Authorization: Bearer koval_tkn_default_admin"
```

---

### 3. Download Optimized Target Binary
`GET /build/{id}/binary`

Downloads the final, hardware-optimized binary packaged as a `.tar.gz` archive.

#### Request Headers
- `Authorization: Bearer koval_tkn_default_admin` (Required)

#### Path Parameters
- **id** (String, Required): The unique UUID of a successfully completed job (e.g. `7f18b456-c392-4911-897b-928efad984d8`).

#### Response Schema
- **200 OK**: Binary archive exists and is downloaded as a `.tar.gz` byte payload.
  - **Headers**:
    - `Content-Type: application/octet-stream`
    - `Content-Disposition: attachment; filename="7f18b456-c392-4911-897b-928efad984d8.tar.gz"`
    - `x-sha256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (SHA-256 integrity checksum).
- **401 Unauthorized**: Missing or invalid authentication token.
- **404 Not Found / 400 Bad Request**: Job not found, failed compilation, or binary is still compiling.

#### Verification Protocol
When integrating with this endpoint, your deployment script must calculate the SHA-256 checksum of the downloaded file and compare it against the value returned in the `x-sha256` header to ensure absolute file integrity before extracting:

```bash
# 1. Download packaged binary archive to /tmp/output.tar.gz
curl -X GET http://localhost:8080/build/7f18b456-c392-4911-897b-928efad984d8/binary \
  -H "Authorization: Bearer koval_tkn_default_admin" \
  -o /tmp/output.tar.gz

# 2. Check sha256 checksum on Linux
sha256sum /tmp/output.tar.gz
```

---

### 4. Webhook Management

Manage webhook notification channels. When build jobs complete (status transitions to `"done"` or `"failed"`), the server POSTs a signed, secure JSON payload to the registered targets.

#### A. Register a Webhook URL
`POST /webhooks`

Registers a new webhook endpoint with a shared HMAC secret.

##### Request Headers
- `Authorization: Bearer <token>` (Required)
- `Content-Type: application/json` (Required)

##### Request Schema (JSON)
- **url** (String, Required): Fully qualified webhook destination HTTP/HTTPS URL.
- **secret** (String, Required): Secret string used for signing payload deliveries.

##### Response Schema
- **201 Created**: Webhook successfully registered.
  ```json
  {
    "id": 1
  }
  ```
- **401 Unauthorized**: Missing or invalid authentication token.

##### Example Command
```bash
curl -X POST http://localhost:8080/webhooks \
  -H "Authorization: Bearer koval_tkn_default_admin" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://ci.example.com/hooks/koval",
    "secret": "my_webhook_secret_key_123"
  }'
```

---

#### B. List Active Webhooks
`GET /webhooks`

Returns an array of registered webhooks associated with the authenticated Bearer token.

##### Request Headers
- `Authorization: Bearer <token>` (Required)

##### Response Schema
- **200 OK**: Active webhooks retrieved. Returns a JSON array of objects:
  - **id** (Integer): Unique ID of the webhook.
  - **url** (String): Webhook destination URL.
  - **created_at** (String): ISO 8601 creation timestamp.
  - **is_active** (Boolean): Deactivation flag state.
  ```json
  [
    {
      "id": 1,
      "url": "https://ci.example.com/hooks/koval",
      "created_at": "2026-05-17T18:00:00Z",
      "is_active": true
    }
  ]
  ```

##### Example Command
```bash
curl -X GET http://localhost:8080/webhooks \
  -H "Authorization: Bearer koval_tkn_default_admin"
```

---

#### C. Deactivate a Webhook
`DELETE /webhooks/{id}`

Deactivates and disables a registered webhook by its identifier.

##### Request Headers
- `Authorization: Bearer <token>` (Required)

##### Path Parameters
- **id** (Integer, Required): The unique ID of the target webhook.

##### Response Schema
- **204 No Content**: Webhook successfully deactivated.
- **401 Unauthorized**: Missing or invalid authorization token.
- **404 Not Found**: No active webhook matches the provided ID, or the webhook belongs to a different token. *Note: The API returns `404 Not Found` rather than `403 Forbidden` to prevent potential webhook presence discovery leaks.*

##### Example Command
```bash
curl -X DELETE http://localhost:8080/webhooks/1 \
  -H "Authorization: Bearer koval_tkn_default_admin"
```

---

#### Webhook Delivery Specification

Whenever a job transitions to `"done"` or `"failed"`, the orchestrator schedules asynchronous HTTP POST deliveries:

##### Delivery Payload (`WebhookPayload`)
* **job_id** (String): Unique build job UUID.
* **status** (String): The final execution state (`"done"` or `"failed"`).
* **finished_at** (String or Null): ISO 8601 completion timestamp.
* **project** (String): Git repository build URL.
* **sha256** (String or Null): Compilation archive SHA-256 hash (only populated when status is `"done"`).

```json
{
  "job_id": "7f18b456-c392-4911-897b-928efad984d8",
  "status": "done",
  "finished_at": "2026-05-17T18:05:00Z",
  "project": "https://github.com/example/project.git",
  "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
}
```

##### Security Signature Header
Every webhook delivery includes the custom signature header `X-Koval-Signature`. The value is computed as:
`X-Koval-Signature: sha256=<hmac>`
where `<hmac>` is the hexadecimal HMAC-SHA256 signature of the raw JSON body payload signed using the registered webhook `secret`.

##### Delivery Retry Policy
If the destination server fails to respond (or returns a status outside the `2xx` range), the orchestrator retries transmission with the following retry pattern:
* **Attempt 1**: Immediate delivery attempt.
* **Attempt 2**: Retried after a **2-second** backoff sleep.
* **Attempt 3**: Retried after a **5-second** backoff sleep.
* After 3 failed attempts, the notification is abandoned.

---

### 5. Token Management

Administrative endpoints to manage access tokens for target compilation environments.

> [!IMPORTANT]
> **Administrative Restrictions:** 
> These endpoints are restricted strictly to personal-use deployments. Plaintext authentication matches must correspond directly to the bootstrapped default administrator token (`koval_tkn_default_admin`). Tokens generated via the API cannot act as administrative roots.

#### A. Create a Bearer Token
`POST /tokens`

Generates and registers a new active client Bearer token.

##### Request Headers
- `Authorization: Bearer koval_tkn_default_admin` (Required)
- `Content-Type: application/json` (Required)

##### Request Schema (JSON)
- **name** (String, Required): Name/identifier for the token (e.g. `"prod-build-box-1"`).

##### Response Schema
- **201 Created**: Token created successfully.
  - **id** (Integer): Database row identifier.
  - **plaintext_token** (String): **Plaintext string displayed ONCE. It is stored in hashed format (bcrypt) and cannot be recovered if lost.**
  - **name** (String): Saved token identifier label.
  ```json
  {
    "id": 2,
    "plaintext_token": "8b7e2840-79ff-4bc0-b0b9-38f382a884fa",
    "name": "prod-build-box-1"
  }
  ```
- **401 Unauthorized**: Missing or invalid authentication token.
- **403 Forbidden**: Access denied — Administrator privileges required.

##### Example Command
```bash
curl -X POST http://localhost:8080/tokens \
  -H "Authorization: Bearer koval_tkn_default_admin" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "prod-build-box-1"
  }'
```

---

#### B. List Registered Tokens
`GET /tokens`

Lists all registered active tokens in the system.

##### Request Headers
- `Authorization: Bearer koval_tkn_default_admin` (Required)

##### Response Schema
- **200 OK**: Active tokens retrieved. Returns a JSON array of objects:
  - **id** (Integer): Token ID.
  - **name** (String): Token identifier label.
  - **created_at** (String): ISO 8601 registration timestamp.
  ```json
  [
    {
      "id": 1,
      "name": "default-admin",
      "created_at": "2026-05-17T18:00:00Z"
    },
    {
      "id": 2,
      "name": "prod-build-box-1",
      "created_at": "2026-05-17T18:05:00Z"
    }
  ]
  ```

##### Example Command
```bash
curl -X GET http://localhost:8080/tokens \
  -H "Authorization: Bearer koval_tkn_default_admin"
```

---

#### C. Revoke a Bearer Token
`DELETE /tokens/{id}`

Revokes/deactivates a registered Bearer token by its ID.

##### Request Headers
- `Authorization: Bearer koval_tkn_default_admin` (Required)

##### Path Parameters
- **id** (Integer, Required): The unique ID of the target token to revoke.

##### Response Schema
- **204 No Content**: Token successfully revoked.
- **401 Unauthorized**: Missing or invalid authentication token.
- **403 Forbidden**: Access denied — Administrator privileges required.

##### Example Command
```bash
curl -X DELETE http://localhost:8080/tokens/2 \
  -H "Authorization: Bearer koval_tkn_default_admin"
```

---

### 6. Job Listing

Query compilation histories.

#### List Recent Compilation Jobs
`GET /jobs`

Retrieves the history of the last 50 build jobs submitted by or visible to the authenticated Bearer token.

##### Request Headers
- `Authorization: Bearer <token>` (Required)

##### Response Schema
- **200 OK**: List of compilation summaries successfully retrieved. Returns a JSON array of objects:
  - **id** (String): Unique job UUID.
  - **project** (String): Project repository URL.
  - **git_ref** (String): Target branch/tag/hash.
  - **status** (String): Compilation state (one of `"queued"`, `"building"`, `"done"`, `"failed"`).
  - **queued_at** (String): ISO 8601 submission timestamp.
  - **started_at** (String or Null): ISO 8601 start timestamp.
  - **finished_at** (String or Null): ISO 8601 completion timestamp.
  ```json
  [
    {
      "id": "7f18b456-c392-4911-897b-928efad984d8",
      "project": "https://github.com/example/project.git",
      "git_ref": "main",
      "status": "done",
      "queued_at": "2026-05-17T18:00:00Z",
      "started_at": "2026-05-17T18:00:05Z",
      "finished_at": "2026-05-17T18:05:00Z"
    }
  ]
  ```

##### Example Command
```bash
curl -X GET http://localhost:8080/jobs \
  -H "Authorization: Bearer koval_tkn_default_admin"
```

---

### 7. Web UI Dashboard

Serves a premium, responsive Web UI dashboard directly from the Axum orchestrator.

#### Serve Web Dashboard Page
`GET /ui`

Serves the standalone HTML/JS single-page application dashboard interface.

##### Request Headers
*No authentication headers required to load the static interface.*

##### Security Configuration
Authentication details (the user's Bearer token) are keyed securely via standard browser fields, stored only inside `sessionStorage` in the client's memory space, and injected as an `Authorization` header dynamically during internal REST fetch requests.

##### Example Command
```bash
# Serves application index HTML to browser clients
curl -X GET http://localhost:8080/ui
```
```
