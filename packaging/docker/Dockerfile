# syntax=docker/dockerfile:1

# --- build stage -------------------------------------------------------
FROM rust:1.82-bookworm AS build
WORKDIR /src
COPY Cargo.toml ./
COPY crates crates
RUN cargo build --release --workspace \
 && strip target/release/drmd

# --- runtime stage -------------------------------------------------------
FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="drmd" \
      org.opencontainers.image.description="DRM O/D/C developmental runtime service" \
      org.opencontainers.image.source="https://github.com/LukeCampbell3/drm-linux-kernel-coder" \
      org.opencontainers.image.licenses="MIT"

RUN apt-get update \
 && apt-get install -y --no-install-recommends coreutils ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /var/lib/drmd --shell /usr/sbin/nologin drmd \
 && mkdir -p /run/drmd /var/lib/drmd \
 && chown -R drmd:drmd /run/drmd /var/lib/drmd

COPY --from=build /src/target/release/drmd /usr/local/bin/drmd

USER drmd
WORKDIR /var/lib/drmd
VOLUME ["/var/lib/drmd"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/drmd", "status", "--socket", "/run/drmd/drmd.sock"]

ENTRYPOINT ["/usr/local/bin/drmd"]
CMD ["serve", "--socket", "/run/drmd/drmd.sock", "--work", "/var/lib/drmd"]
