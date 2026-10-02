# syntax=docker/dockerfile:1
FROM ghcr.io/sebadob/rauthy-builder:20260519@sha256:6bea5d732ebfa4613c51a55c1c9c8b5c59dddfde9adbb6eca899d6599fd2b219 AS build

WORKDIR /work
COPY . .

RUN --mount=type=cache,target=/root/.npm \
    cd frontend && npm ci --no-audit --no-fund

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cd src/wasm-modules \
    && wasm-pack build -d ../../frontend/src/wasm/spow --no-pack --out-name spow --features spow \
    && wasm-pack build -d ../../frontend/src/wasm/md --no-pack --out-name md --features md

RUN mkdir -p templates/html static/v1 && cd frontend && npm run build

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/work/target \
    cargo build --release --locked --target x86_64-unknown-linux-gnu \
    && mkdir -p /out/empty \
    && cp target/x86_64-unknown-linux-gnu/release/rauthy /out/rauthy

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f

ARG TARGET_USER="10001:10001"

LABEL org.opencontainers.image.source="https://github.com/jhash/rauthy"
LABEL org.opencontainers.image.description="Rauthy with the mili patches"
LABEL org.opencontainers.image.licenses="Apache-2.0"

USER $TARGET_USER
ENV THEME_CUSTOM_DIR=/app/theme
ENV MALLOC_CONF="abort_conf:true,narenas:8,tcache_max:4096,dirty_decay_ms:5000,muzzy_decay_ms:5000"
WORKDIR /app

COPY --from=build --chown=$TARGET_USER /out/rauthy ./rauthy
COPY --from=build --chown=$TARGET_USER /work/config-local-test.toml ./config-local-test.toml
COPY --from=build --chown=$TARGET_USER /out/empty/ ./data
COPY --from=build --chown=$TARGET_USER /out/empty/ ./tls
COPY --chown=$TARGET_USER deploy/mili/theme/ ./theme

ENTRYPOINT ["/app/rauthy"]
CMD ["serve"]
