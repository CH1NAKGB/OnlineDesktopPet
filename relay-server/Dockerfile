FROM rust:1.98-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY shared-protocol ./shared-protocol
COPY relay-server ./relay-server
COPY pet-client ./pet-client
RUN cargo build --release -p relay-server

FROM debian:bookworm-slim
RUN useradd --system --create-home chick
COPY --from=build /src/target/release/relay-server /usr/local/bin/relay-server
USER chick
EXPOSE 8080
ENV BIND_ADDR=0.0.0.0:8080
ENTRYPOINT ["relay-server"]
