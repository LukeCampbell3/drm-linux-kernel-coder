FROM gcc:14-bookworm AS build
WORKDIR /src
COPY . .
RUN cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release && cmake --build build && ctest --test-dir build --output-on-failure
ENTRYPOINT ["/src/build/drm_bytecode"]
CMD ["--out", "/tmp/drm-results"]
