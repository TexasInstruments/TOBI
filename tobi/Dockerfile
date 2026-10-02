FROM rust:1.94-bookworm AS build

WORKDIR /app/tobi
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config liblzma-dev \
    && rm -rf /var/lib/apt/lists/*

COPY tobi/Cargo.toml tobi/Cargo.lock* ./
COPY tobi/src ./src
COPY catalog.json /app/catalog.json
COPY docs/boot-guide.json /app/docs/boot-guide.json
RUN cargo build --release

FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates liblzma5 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /app/tobi/target/release/tobi /usr/local/bin/tobi
COPY catalog.json ./catalog.json
COPY LICENSE /usr/share/doc/tobi/LICENSE

ENTRYPOINT ["tobi"]
CMD ["--mode", "mock"]
