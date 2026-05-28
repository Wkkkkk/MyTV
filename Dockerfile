# syntax=docker/dockerfile:1

# ── Stage 1: build ──────────────────────────────────────────────────────────
FROM rust:1-slim AS builder

WORKDIR /app

# Install the Charles Proxy CA cert so cargo can reach crates.io through the
# corporate SSL proxy.  This cert is only needed at build time on this machine.
COPY charles-proxy-ca.pem /usr/local/share/ca-certificates/charles-proxy-ca.crt
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && update-ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Build dependencies layer (cached unless Cargo.toml/Cargo.lock change)
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Build the real binary (migrations dir required by sqlx::migrate! at compile time;
# templates dir required by askama at compile time)
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
RUN touch src/main.rs && cargo build --release

# ── Stage 2: runtime ────────────────────────────────────────────────────────
FROM debian:trixie-slim AS runtime

WORKDIR /app

# ca-certificates: needed by reqwest (rustls) for HTTPS
# python3-pip: used to install yt-dlp (avoids binary curl download that breaks on SSL proxies)
# The Charles Proxy CA cert is injected so pip can reach PyPI through the corporate SSL proxy.
COPY charles-proxy-ca.pem /usr/local/share/ca-certificates/charles-proxy-ca.crt
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates python3-pip \
    && update-ca-certificates \
    && pip3 install --break-system-packages yt-dlp \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/mytv ./mytv

ENV PORT=8080
ENV RUST_LOG=info

EXPOSE 8080

CMD ["./mytv"]
