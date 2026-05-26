#!/bin/sh
set -e

# Configured variables replaced by the Koval server during serving:
SERVER_URL="http://localhost:8090"
PROJECT="/koval"
REF="main"
TOKEN="koval_tkn_default_admin"

echo "======================================================"
echo " Starting Koval Smart Installer for ${PROJECT} (${REF})"
echo "======================================================"

# 1. Detect architecture
OS=$(uname -s)
ARCH=$(uname -m)

if [ "$OS" != "Linux" ]; then
    echo "Error: Only Linux is supported at the moment." >&2
    exit 1
fi

case "$ARCH" in
    x86_64)
        TARGET_ARCH="x86_64"
        ;;
    aarch64)
        TARGET_ARCH="aarch64"
        ;;
    *)
        echo "Error: Unsupported architecture: ${ARCH}" >&2
        exit 1
        ;;
esac

# Create a secure temp directory
TMP_DIR=$(mktemp -d -t koval-install-XXXXXX)
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

echo "--> Downloading static hardware probe for ${TARGET_ARCH}..."
PROBE_PATH="${TMP_DIR}/koval-probe"

# Check if curl or wget is available
if command -v curl >/dev/null 2>&1; then
    curl -sSfL "${SERVER_URL}/probe/static/${TARGET_ARCH}" -o "$PROBE_PATH"
elif command -v wget >/dev/null 2>&1; then
    wget -qO "$PROBE_PATH" "${SERVER_URL}/probe/static/${TARGET_ARCH}"
else
    echo "Error: Neither curl nor wget was found. Please install one of them." >&2
    exit 1
fi

chmod +x "$PROBE_PATH"

echo "--> Profiling host hardware..."
# Run the probe and save the profile
PROFILE_PATH="${TMP_DIR}/profile.json"
"$PROBE_PATH" > "$PROFILE_PATH"

# Send the profile to Koval server and get build status
echo "--> Requesting optimal build from Koval Server..."
if command -v curl >/dev/null 2>&1; then
    RESPONSE=$(curl -sSf -X POST \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${TOKEN}" \
        -d @"$PROFILE_PATH" \
        "${SERVER_URL}/forge/install?project=${PROJECT}&ref=${REF}")
else
    RESPONSE=$(wget -qO- --post-file="$PROFILE_PATH" \
        --header="Content-Type: application/json" \
        --header="Authorization: Bearer ${TOKEN}" \
        "${SERVER_URL}/forge/install?project=${PROJECT}&ref=${REF}")
fi

STATUS=$(echo "$RESPONSE" | grep -o '"status":"[^"]*' | grep -o '[^"]*$')

if [ "$STATUS" = "cached" ]; then
    DOWNLOAD_URL=$(echo "$RESPONSE" | grep -o '"download_url":"[^"]*' | grep -o '[^"]*$')
    EXPECTED_SHA=$(echo "$RESPONSE" | grep -o '"sha256":"[^"]*' | grep -o '[^"]*$')
    echo "--> Found optimized cached build!"
elif [ "$STATUS" = "building" ]; then
    JOB_ID=$(echo "$RESPONSE" | grep -o '"job_id":"[^"]*' | grep -o '[^"]*$')
    echo "--> Build not cached. Enqueued build job: ${JOB_ID}"
    echo "--> Waiting for compilation to finish..."
    
    while true; do
        sleep 2
        if command -v curl >/dev/null 2>&1; then
            STATUS_RESP=$(curl -sSf -H "Authorization: Bearer ${TOKEN}" "${SERVER_URL}/build/${JOB_ID}/status")
        else
            STATUS_RESP=$(wget -qO- --header="Authorization: Bearer ${TOKEN}" "${SERVER_URL}/build/${JOB_ID}/status")
        fi
        
        JOB_STATUS=$(echo "$STATUS_RESP" | grep -o '"status":"[^"]*' | grep -o '[^"]*$')
        
        if [ "$JOB_STATUS" = "completed" ] || [ "$JOB_STATUS" = "done" ]; then
            DOWNLOAD_URL="/build/${JOB_ID}/binary"
            # Get expected SHA from status response
            EXPECTED_SHA=$(echo "$STATUS_RESP" | grep -o '"artifact_sha256":"[^"]*' | grep -o '[^"]*$')
            echo "--> Build completed successfully!"
            break
        elif [ "$JOB_STATUS" = "failed" ]; then
            echo "Error: Compilation job failed. Check server logs." >&2
            exit 1
        else
            echo "--> Build in progress (status: ${JOB_STATUS})...."
        fi
    done
else
    echo "Error: Unexpected response from server: ${RESPONSE}" >&2
    exit 1
fi

# Ensure DOWNLOAD_URL is full URL
case "$DOWNLOAD_URL" in
    http://*|https://*)
        FULL_DOWNLOAD_URL="$DOWNLOAD_URL"
        ;;
    *)
        FULL_DOWNLOAD_URL="${SERVER_URL}${DOWNLOAD_URL}"
        ;;
esac

echo "--> Downloading optimized binary archive..."
BIN_PATH="${TMP_DIR}/archive.tar.gz"
if command -v curl >/dev/null 2>&1; then
    curl -sSfL -H "Authorization: Bearer ${TOKEN}" "$FULL_DOWNLOAD_URL" -o "$BIN_PATH"
else
    wget -qO "$BIN_PATH" --header="Authorization: Bearer ${TOKEN}" "$FULL_DOWNLOAD_URL"
fi

# Verify SHA256 checksum if coreutils / sha256sum exists
if [ -n "$EXPECTED_SHA" ] && [ "$EXPECTED_SHA" != "null" ]; then
    if command -v sha256sum >/dev/null 2>&1; then
        echo "--> Verifying checksum..."
        ACTUAL_SHA=$(sha256sum "$BIN_PATH" | awk '{print $1}')
        if [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
            echo "Error: Checksum verification failed." >&2
            echo "Expected: ${EXPECTED_SHA}" >&2
            echo "Actual:   ${ACTUAL_SHA}" >&2
            exit 1
        fi
        echo "--> Checksum matches!"
    fi
fi

echo "--> Extracting archive..."
EXTRACT_DIR="${TMP_DIR}/extracted"
mkdir -p "$EXTRACT_DIR"
tar -xzf "$BIN_PATH" -C "$EXTRACT_DIR"

# Ask user where to install
INSTALL_DIR=""
echo ""
printf "Install system-wide to /usr/local/bin (requires sudo)? [y/N]: "
read -r CONFIRM

USE_SUDO="false"
if [ "$CONFIRM" = "y" ] || [ "$CONFIRM" = "Y" ]; then
    INSTALL_DIR="/usr/local/bin"
    USE_SUDO="true"
else
    INSTALL_DIR="${HOME}/.local/bin"
    mkdir -p "$INSTALL_DIR"
fi

echo "--> Installing binaries to ${INSTALL_DIR}..."
for f in "$EXTRACT_DIR"/*; do
    if [ -f "$f" ]; then
        fname=$(basename "$f")
        chmod +x "$f"
        if [ "$USE_SUDO" = "true" ]; then
            echo "--> Installing ${fname} to ${INSTALL_DIR} (using sudo)..."
            sudo cp "$f" "${INSTALL_DIR}/${fname}"
        else
            echo "--> Installing ${fname} to ${INSTALL_DIR}..."
            cp "$f" "${INSTALL_DIR}/${fname}"
        fi
        echo "--> Successfully installed: ${INSTALL_DIR}/${fname}"
    fi
done

# Check if INSTALL_DIR is in PATH
if [ "$USE_SUDO" = "false" ]; then
    case ":${PATH}:" in
        *:"${INSTALL_DIR}":*)
            ;;
        *)
            echo ""
            echo "WARNING: ${INSTALL_DIR} is not in your PATH environment variable."
            echo "To run installed command(s), add this to your shell profile (e.g. ~/.bashrc or ~/.zshrc):"
            echo "  export PATH=\"\$PATH:${INSTALL_DIR}\""
            ;;
    esac
fi

echo ""
echo "Installation complete!"
