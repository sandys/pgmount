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

# Stage 2: Runtime image extending the upstream OpenShell base sandbox
FROM ghcr.io/nvidia/openshell-community/sandboxes/base:latest

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

# Create mount points and declare supervisor-managed FUSE mounts.
# OpenShell overrides the container command to run `openshell-sandbox`, which
# reads `/etc/fstab` and mounts `fuse.*` entries marked `noauto` before
# spawning the sandboxed child process.
RUN mkdir -p /db /home/agent && \
    printf '%s\n' \
      'env /db fuse.openeral ro,allow_other,noauto 0 0' \
      'env#workspace#${OPENSHELL_SANDBOX_ID} /home/agent fuse.openeral rw,allow_other,noauto 0 0' \
      >> /etc/fstab

# StringCost integration: allow Claude to reach the StringCost proxy for
# cost-tracked API calls when ANTHROPIC_BASE_URL points at StringCost.
RUN cat >> /etc/openshell/policy.yaml <<'POLICY'

  stringcost:
    name: stringcost
    endpoints:
      - { host: proxy.stringcost.com, port: 443 }
    binaries:
      - { path: /usr/local/bin/claude }
      - { path: /usr/bin/node }
POLICY

# Ship project-local skills into the sandbox home.
COPY .claude/skills/ /sandbox/.skills/
