# ==========================================
# STAGE 1: Builder
# ==========================================
FROM rust:latest AS builder

WORKDIR /koval-build

# Copy entire project sources
COPY . .

# Build production release binary for the server
RUN cargo build --release -p server

# ==========================================
# STAGE 2: Runtime
# ==========================================
FROM rust:latest AS runtime

# Install system dependencies needed for runtime git-cloning, packaging, and database management
RUN apt-get update && apt-get install -y \
    sqlite3 \
    libsqlite3-dev \
    git \
    tar \
    && rm -rf /var/lib/apt/lists/*

# Set working directory for the application executable
WORKDIR /koval

# Copy release compiled binaries from builder stage
COPY --from=builder /koval-build/target/release/server /koval/server

# Create dedicated persistent storage directories for Koval
RUN mkdir -p /koval/db /koval/artifacts

# Configure environmental variables pointing to persistent paths
ENV KOVAL_DB=/koval/db/koval.db
ENV KOVAL_ARTIFACTS_DIR=/koval/artifacts
ENV KOVAL_QUEUE_CAPACITY=10
ENV KOVAL_RATE_LIMIT=20
ENV KOVAL_PORT=8731

# Expose Koval Server's listening port
EXPOSE 8731

# Define persistent volume mount points for operational safety
VOLUME ["/koval/db", "/koval/artifacts"]

# Set binary execution entrypoint
ENTRYPOINT ["/koval/server"]
