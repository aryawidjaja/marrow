# Build the Marrow backbone straight from the public repo, so this file is self-contained:
#   docker build -f deploy/Dockerfile -t marrow-backbone deploy    (context doesn't matter)
#   or just `fly deploy` from this deploy/ folder.
# Pin a version with --build-arg MARROW_REF=v0.2.0 (defaults to the latest main).
FROM rust:1-slim AS build
RUN apt-get update && apt-get install -y --no-install-recommends git gcc libc6-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
ARG MARROW_REF=main
RUN git clone --depth 1 --branch ${MARROW_REF} https://github.com/aryawidjaja/marrow /src
WORKDIR /src
RUN cargo build --release -p marrow-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m marrow && mkdir -p /data && chown marrow:marrow /data
COPY --from=build /src/target/release/marrow-server /usr/local/bin/marrow-server
USER marrow
ENV MARROW_DATA=/data PORT=8787
VOLUME /data
EXPOSE 8787
CMD ["marrow-server"]
