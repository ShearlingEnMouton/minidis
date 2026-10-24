FROM rust:1-bookworm AS builder
WORKDIR /app

RUN mkdir -p /data

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

COPY src ./src
RUN touch src/main.rs && cargo build --release

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /app/target/release/minidis /usr/local/bin/minidis
USER nonroot
COPY --from=builder --chown=nonroot:nonroot /data /data
ENV AOF_PATH="/data/appendonly.aof"
EXPOSE 3131
ENTRYPOINT ["minidis"]
