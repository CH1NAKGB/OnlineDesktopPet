FROM rust:1.98-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY shared-protocol ./shared-protocol
COPY relay-server ./relay-server
COPY pet-client ./pet-client
RUN cargo build --locked --release -p relay-server -j 1

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home chick
COPY --from=build /src/target/release/relay-server /usr/local/bin/relay-server
USER chick
EXPOSE 8080
ENV BIND_ADDR=0.0.0.0:8080
ENTRYPOINT ["relay-server"]
