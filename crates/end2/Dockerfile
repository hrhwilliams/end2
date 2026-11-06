FROM rust:1.99-slim-trixie AS chef
WORKDIR /build
RUN cargo install cargo-chef

FROM chef AS planner
COPY ./crates/end2 ./end2
RUN echo '[workspace]\nmembers = ["end2"]\nresolver = "3"' > Cargo.toml
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS backend-builder
WORKDIR /build
RUN apt update && apt install -y linux-perf libpq-dev openssl libssl-dev pkg-config
RUN cargo install diesel_cli --no-default-features --features postgres
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY ./crates/end2 /build/end2
RUN cargo build --release

FROM debian:trixie-slim AS runtime
RUN apt-get  update && apt-get install -y linux-perf postgresql
WORKDIR /app
COPY --from=backend-builder /build/target/release/end2 .
COPY --from=backend-builder /build/target/release/gen_keys .
COPY --from=backend-builder /usr/local/cargo/bin/diesel /usr/local/bin/diesel
COPY --from=planner /build/end2/migrations /app/migrations
COPY --from=planner /build/end2/entrypoint.sh .
ENTRYPOINT [ "./entrypoint.sh" ]
CMD ["./end2"]