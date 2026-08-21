# Reproducible peer-test container for the Rust DRM O/D/C runtime.
# The runtime has no crates.io dependencies; only the pinned Rust toolchain image is needed to build it.
FROM rust:1.94.0-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55 AS build
WORKDIR /src
COPY Cargo.toml ./
COPY src ./src
RUN cargo test --release && cargo build --release

FROM debian:bookworm-20260803-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241
COPY --from=build /src/target/release/drm-live-odc /usr/local/bin/drm-live-odc
RUN mkdir -p /results && chown 10001:10001 /results
USER 10001:10001
ENV DRM_OUT=/results
ENTRYPOINT ["/usr/local/bin/drm-live-odc"]
