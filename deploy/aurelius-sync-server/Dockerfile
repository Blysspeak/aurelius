# Build context MUST be the repository root (workspace), not this directory —
# aurelius-sync-server depends on aurelius-core via a path dependency, and the
# workspace's Cargo.toml lists all crates, so the whole workspace tree is
# needed to resolve. See ../../deploy/aurelius-sync-server/docker-compose.yml
# for the matching `context: ../..` / `dockerfile:` pair.
#
#   docker build -f deploy/aurelius-sync-server/Dockerfile -t aurelius-sync-server .

# ---- Build stage ----
FROM rust:1-slim-bookworm AS builder
WORKDIR /app

# rusqlite's "bundled" feature compiles SQLite from source (needs a C
# toolchain); reqwest (an aurelius-core dependency) needs OpenSSL headers.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo build --release -p aurelius-sync-server

# ---- Runtime stage ----
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/aurelius-sync-server /usr/local/bin/aurelius-sync-server

# The server's own SQLite file lives here; mount a volume over it (see
# docker-compose.yml) so data survives container recreation.
VOLUME ["/data"]
EXPOSE 8181

ENTRYPOINT ["aurelius-sync-server"]
CMD ["--port", "8181", "--db", "/data/aurelius-sync-server.db"]
