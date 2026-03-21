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

# Stage 2: Runtime shell
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    fuse3 \
    libpq5 \
    bash \
    curl \
    git \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy openeral binary
COPY --from=builder /build/target/release/openeral /usr/local/bin/openeral

# Allow non-root FUSE mounts
RUN echo "user_allow_other" >> /etc/fuse.conf

# Create mount points
RUN mkdir -p /db /home/agent

# Copy entrypoint
COPY openeral-shell/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Copy skills (available to agents inside the shell)
COPY openeral-shell/skills/ /etc/openeral/skills/

WORKDIR /home/agent

ENTRYPOINT ["entrypoint.sh"]
CMD ["sleep", "infinity"]
