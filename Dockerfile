# ── Stage 1: build ─────────────────────────────────────────────────────────────
FROM rust:1.87-slim AS builder

WORKDIR /app

# Install system dependencies needed for compilation
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

# Clone the repository from GitHub
RUN git clone https://github.com/VMASPAD/port-repeat.git . && \
    git config --global --add safe.directory /app

# Build the port-repeat-server binary
RUN cargo build --release -p port-repeat-server

# ── Stage 2: runtime ────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/port-repeat-server /usr/local/bin/port-repeat-server

# Copy config from cloned repository
COPY --from=builder /app/config/server.toml ./config/server.toml

# Control port (configure actual tunnel ports in your server.toml and expose them too)
EXPOSE 7000

ENTRYPOINT ["port-repeat-server", "--config", "config/server.toml"]
