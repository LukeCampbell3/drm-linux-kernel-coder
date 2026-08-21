FROM gcc:14.2.0-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends cmake ninja-build && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY CMakeLists.txt .
COPY src src
RUN cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release && cmake --build build && ctest --test-dir build --output-on-failure

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends coreutils ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/build/drm_odc_live /usr/local/bin/drm_odc_live
ENTRYPOINT ["/usr/local/bin/drm_odc_live"]
CMD ["--out","/results"]
