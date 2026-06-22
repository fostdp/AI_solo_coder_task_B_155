# syntax=docker/dockerfile:1.6

FROM rust:1.77-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /src

FROM chef AS planner
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
COPY backend/config ./config
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
COPY backend/config ./config
RUN cargo build --release --bin urn-acoustics-backend

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && update-ca-certificates

RUN groupadd -r urn && useradd -r -g urn urn

WORKDIR /app
COPY --from=builder /src/target/release/urn-acoustics-backend /app/urn-acoustics-backend
COPY backend/config /app/config
COPY frontend /app/static
COPY database /app/database

RUN chown -R urn:urn /app
USER urn

ENV URN_CONFIG_DIR=/app/config
ENV RUST_LOG=urn_acoustics_backend=info,tower_http=info,axum=info

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=3 \
    CMD curl -f http://localhost:8080/api/health || exit 1

ENTRYPOINT ["/app/urn-acoustics-backend"]
