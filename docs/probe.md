# Koval Probe Deployment Guide

**Target Audience:** User setting up probe on a target device.

Welcome! This guide will help you set up and run the Koval hardware probe on your target device. You do not need to know how to write Rust code to follow these instructions — we will guide you step-by-step through terminal commands.

---

## 1. What is the Koval Probe?

The Koval probe is a small, lightweight system diagnostics utility. It must run directly on the **target client hardware** where your final software will be deployed, rather than on the central build server. 

By running on the target machine, it safely collects precise hardware measurements — such as active CPU features, actual RAM capacities, direct file system throughput (via O_DIRECT), and local graphics card capabilities. It packages these metrics into a clean JSON structure that the build server uses to compile a custom, highly optimized binary tailored specifically for this machine.

---

## 2. Prerequisites

The probe binary has very minimal dependencies:
- **Operating System**: Linux kernel 5.1 or newer.
- **Architecture**: Linux x86_64 or aarch64.
- **Graphics (Optional)**: If you want the probe to detect device-local GPU configurations and VRAM sizes, ensure the Vulkan driver client library is installed (`libvulkan1` on Debian/Ubuntu systems).

---

## 3. How to Get the Probe Binary

Since the central Koval build coordinator is dedicated purely to compilation workflows and database scheduling, the probe utility is distributed out-of-band or built directly from source.

### Option A: Local Compilation on Target (Recommended)
If your target machine has the Rust toolchain installed, clone the repository and build the probe locally in release mode:

```bash
# Compile the probe in release mode
cargo build -p probe --release

# The compiled binary is generated at:
# target/release/probe
```

### Option B: Deploy Precompiled Executable
If you are deploying to clean target nodes without compilers:
1. Build the binary on an identical architectural staging node using:
   ```bash
   cargo build -p probe --release
   ```
2. Distribute the generated `target/release/probe` executable to target servers using secure copy (`scp`):
   ```bash
   scp target/release/probe user@192.168.1.50:/tmp/probe
   ```

---

## 4. Running the Probe

To inspect the system and output the hardware profile, simply run the compiled executable in your terminal:

```bash
/tmp/probe
```

### Output Example
The probe will inspect your hardware subsystems and print a single formatted JSON block to your stdout:

```json
{
  "cpu": {
    "flags": [
      "avx",
      "avx2",
      "sse",
      "sse2",
      "sse4.1"
    ],
    "cache_topology": "L1:32KB, L2:256KB, L3:12MB",
    "core_count": 8
  },
  "memory": {
    "total_bytes": 17179869184,
    "available_bytes": 12884901888,
    "bandwidth_mbs": 21500.0
  },
  "storage": {
    "io_uring": true,
    "o_direct": true,
    "read_speed_mbs": 520.4,
    "write_speed_mbs": 460.1
  },
  "gpu": {
    "devices": [
      {
        "name": "NVIDIA GeForce RTX 4060",
        "vram_bytes": 8589934592,
        "pcie_info": "PCIe Link: x8 @ 16.0 GT/s"
      }
    ]
  }
}
```

---

## 5. Submitting Your Profile to the Build Server

To trigger a custom build utilizing your collected hardware characteristics, redirect the probe's stdout directly into a `curl` build payload sent to the orchestrator:

```bash
# Capture profile and POST to the coordinator to compile the project
curl -X POST http://localhost:8080/build \
  -H "Authorization: Bearer koval_tkn_default_admin" \
  -H "Content-Type: application/json" \
  -d "{
    \"hardware\": $(./target/release/probe),
    \"project\": \"https://github.com/example/project.git\",
    \"git_ref\": \"main\"
  }"
```

---

## 6. Vulkan Fail-Safe & Fallbacks

If your target device does not have a graphics card or the Vulkan ICD drivers (`libvulkan.so.1`) installed, **the probe will not crash or panic**.

- **Dynamic Loading Check**: The probe uses the standard dynamic library loading pattern (`ash::Entry::load()`). If the Vulkan runtime is missing, it catches the failure gracefully.
- **DRM Sysfs Fallback**: Upon failing to load Vulkan, the probe immediately attempts to parse system cards registered in the Linux kernel via `/sys/class/drm/`.
- **Output Changes**: If no GPU is found or Vulkan is missing, the `gpu` object in the JSON output will report a DRM graphics placeholder with 0 bytes of VRAM, ensuring that compilation mapping rules evaluate safely without crashing:

```json
  "gpu": {
    "devices": [
      {
        "name": "DRM Graphics Device (card0)",
        "vram_bytes": 0,
        "pcie_info": "PCIe Link: x16 @ 8.0 GT/s"
      }
    ]
  }
```
