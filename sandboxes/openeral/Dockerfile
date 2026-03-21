# Stage 1: Build openeral from source
FROM rust:1.85-bookworm AS builder

RUN apt-get update && apt-get install -y \
    libfuse3-dev \
    fuse3 \
    pkg-config \
    libpq-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

RUN cargo build --release --bin openeral \
    && strip /build/target/release/openeral

# Stage 2: Runtime image extending the OpenClaw sandbox
FROM ghcr.io/openshell-community/sandbox-openclaw:latest

USER root

# Install only runtime dependencies (no -dev packages)
RUN apt-get update && apt-get install -y --no-install-recommends \
    fuse3 \
    libpq5 \
    && rm -rf /var/lib/apt/lists/*

# Copy openeral binary
COPY --from=builder /build/target/release/openeral /usr/local/bin/openeral

# Allow non-root FUSE mounts
RUN echo "user_allow_other" >> /etc/fuse.conf

# Create mount points
RUN mkdir -p /db /home/agent

# Copy entrypoint and skills
COPY sandboxes/openeral/openeral-start.sh /usr/local/bin/openeral-start.sh
RUN chmod +x /usr/local/bin/openeral-start.sh

COPY sandboxes/openeral/skills/ /sandbox/.skills/

ENTRYPOINT ["openeral-start.sh"]
