# Base image for the runtime stage
ARG BASE_IMAGE=ghcr.io/nvidia/openshell-community/sandboxes/base:latest

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

# Stage 2: Runtime — extends OpenShell base sandbox
FROM ${BASE_IMAGE}

USER root

# Runtime deps for openeral (FUSE + PostgreSQL client lib)
RUN apt-get update && apt-get install -y --no-install-recommends \
    fuse3 \
    libpq5 \
    && rm -rf /var/lib/apt/lists/*

# Copy openeral binary
COPY --from=builder /build/target/release/openeral /usr/local/bin/openeral

# Allow non-root FUSE mounts
RUN echo "user_allow_other" >> /etc/fuse.conf

# Create mount points
RUN mkdir -p /db /home/agent && chown sandbox:sandbox /home/agent

# Copy security policy
COPY openeral-shell/policy.yaml /etc/openshell/policy.yaml

# Copy startup script
COPY openeral-shell/openeral-shell-start.sh /usr/local/bin/openeral-shell-start
RUN chmod +x /usr/local/bin/openeral-shell-start

# Copy skills to standard OpenShell location
COPY openeral-shell/skills/ /sandbox/.agents/skills/

# Local config dir for Claude Code (avoids FUSE config watcher race)
RUN mkdir -p /sandbox/.openeral-config && chown sandbox:sandbox /sandbox/.openeral-config

USER sandbox

ENTRYPOINT ["/bin/bash"]
