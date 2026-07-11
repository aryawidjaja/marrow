# Build the Marrow backbone and ship it on a slim runtime.
# Build context is the repo root:  docker build -f deploy/Dockerfile -t marrow-backbone .
FROM rust:1-slim AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends gcc libc6-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p marrow-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* && useradd -m marrow
COPY --from=build /src/target/release/marrow-server /usr/local/bin/marrow-server
USER marrow
ENV MARROW_DATA=/data PORT=8787
VOLUME /data
EXPOSE 8787
CMD ["marrow-server"]
