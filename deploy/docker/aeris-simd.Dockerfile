# syntax=docker/dockerfile:1
# Reproducible build on the pinned toolchain; non-root runtime.
FROM rust:1.100.0-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p aeris-simd \
 && cp target/release/aeris-simd /aeris-simd

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home aeris
COPY --from=build /aeris-simd /usr/local/bin/aeris-simd
USER 10001
ENTRYPOINT ["/usr/local/bin/aeris-simd"]
CMD ["run"]
