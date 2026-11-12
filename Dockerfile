# syntax=docker/dockerfile:1.7

FROM node:22-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.93-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --release -p doonce-cli

FROM debian:bookworm-slim AS runtime
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates chromium curl tini \
    && rm -rf /var/lib/apt/lists/* \
    && printf '#!/bin/sh\nexec /usr/bin/chromium --no-sandbox --disable-gpu "$@"\n' > /usr/local/bin/chromium-no-sandbox \
    && chmod 0755 /usr/local/bin/chromium-no-sandbox \
    && useradd --uid 10001 --create-home --home-dir /home/doonce --shell /usr/sbin/nologin doonce \
    && mkdir -p /app /data/artifacts /home/doonce \
    && chown -R doonce:doonce /app /data /home/doonce

WORKDIR /app
COPY --from=build /src/target/release/doonce /usr/local/bin/doonce
COPY --from=web /src/web/dist /app/web
COPY deploy/docker/entrypoint.sh /usr/local/bin/doonce-entrypoint
RUN chmod 0755 /usr/local/bin/doonce-entrypoint

ENV DOONCE_HOME=/data \
    DOONCE_UI_DIR=/app/web \
    DOONCE_ARTIFACT_DIR=/data/artifacts \
    DOONCE_CHROME=/usr/local/bin/chromium-no-sandbox \
    DOONCE_BIND=0.0.0.0:8080 \
    DOONCE_LOG=info \
    DOONCE_TELEMETRY=false

USER doonce
EXPOSE 8080
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8080/api/v1/health || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/doonce-entrypoint"]
CMD ["serve"]
