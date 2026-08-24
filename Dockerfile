# syntax=docker/dockerfile:1

# ── Stage 1: build ──────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder

WORKDIR /app

# Build dependencies layer (cached unless Cargo.toml/Cargo.lock change).
# The dummy bench satisfies the [[bench]] manifest entry and stays in place:
# cargo build never compiles bench targets, it only resolves their paths.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src benches && echo 'fn main() {}' > src/main.rs \
    && echo 'fn main() {}' > benches/hot_paths.rs \
    && cargo build --release \
    && rm -rf src

# Build the real binary (migrations dir required by sqlx::migrate! at compile time;
# templates dir required by askama at compile time)
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
COPY static ./static
RUN touch src/main.rs && cargo build --release
RUN strip /app/target/release/mytv

# ── Stage 2: runtime ────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# ca-certificates: needed by reqwest (rustls) for HTTPS
# python3-pip: used to install yt-dlp (called via Command::new("yt-dlp") at runtime)
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates python3-pip \
    && update-ca-certificates \
    && pip3 install --break-system-packages yt-dlp==2026.8.19 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/mytv ./mytv

RUN useradd -r -u 1001 -U appuser
USER appuser

ENV PORT=8080
ENV RUST_LOG=info

EXPOSE 8080

CMD ["./mytv"]
