# Stage 1: Build pgmount from source
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

RUN cargo build --release --bin pgmount \
    && strip /build/target/release/pgmount

# Stage 2: Runtime image extending the OpenClaw sandbox
FROM ghcr.io/openshell-community/sandbox-openclaw:latest

USER root

# Install only runtime dependencies (no -dev packages)
RUN apt-get update && apt-get install -y --no-install-recommends \
    fuse3 \
    libpq5 \
    && rm -rf /var/lib/apt/lists/*

# Copy pgmount binary
COPY --from=builder /build/target/release/pgmount /usr/local/bin/pgmount

# Allow non-root FUSE mounts
RUN echo "user_allow_other" >> /etc/fuse.conf

# Create mount point
RUN mkdir -p /db

# Copy entrypoint and skills
COPY sandboxes/pgmount/pgmount-start.sh /usr/local/bin/pgmount-start.sh
RUN chmod +x /usr/local/bin/pgmount-start.sh

COPY sandboxes/pgmount/skills/ /sandbox/.skills/

ENTRYPOINT ["pgmount-start.sh"]
