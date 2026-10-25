# minidis

minidis is a small Redis-compatible server written in Rust. It speaks RESP2 over TCP, so `redis-cli` and most Redis client libraries can connect to it. It is a learning project and implements a limited set of Redis commands, not all of them.

What it does:

- Stores strings, lists and hashes in memory
- Expires keys after a TTL (checked on access, plus a background cleanup)
- Saves writes to an append-only file (AOF) and replays it on startup
- Ships as a Docker image and a Helm chart on GitHub Container Registry (GHCR)

## Supported commands

| Group | Commands |
| --- | --- |
| Connection | `PING [message]` |
| Strings | `GET`, `SET key value [EX seconds \| PX milliseconds \| EXAT unix-seconds \| PXAT unix-milliseconds]` |
| Keys | `DEL key [key ...]`, `TYPE`, `DBSIZE` |
| Expiry | `TTL`, `PTTL`, `EXPIRE`, `PEXPIRE`, `PEXPIREAT`, `PERSIST` |
| Lists | `LPUSH`, `RPUSH`, `LPOP`, `RPOP`, `LLEN`, `LRANGE` |
| Hashes | `HSET`, `HGET`, `HDEL`, `HLEN`, `HEXISTS`, `HGETALL` |
| Server | `REWRITEAOF` |

Using a command on a key of the wrong type returns `WRONGTYPE`, like Redis does.

Not supported:

- Options not listed above, such as `SET ... NX/XX/GET/KEEPTTL`, `EXPIRE ... NX/XX/GT/LT`, and the count argument of `LPOP`/`RPOP`
- Authentication, replication, pub/sub, transactions, and multiple databases

## Quick start

Building requires Rust 1.85 or newer (the crate uses edition 2024). `redis-cli` is useful for trying the server out.

```sh
AOF_PATH=./data/appendonly.aof cargo run --release
```

The server listens on port 3131. In another terminal:

```sh
redis-cli -p 3131 set greeting hello EX 60
redis-cli -p 3131 get greeting
```

`AOF_PATH` has to be a path the server can write to. Missing parent directories are created.

## Configuration

All settings are environment variables.

| Variable | Default | Description |
| --- | --- | --- |
| `BIND_ADDR` | `0.0.0.0` | Address to listen on |
| `EXPOSE_PORT` | `3131` | Port to listen on |
| `LOG_LEVEL` | `INFO` | One of `ERR`, `WARN`, `INFO`, `DEBUG`. Other values fall back to `INFO` |
| `PURGE_LIMIT` | `1000` | Most expired keys the background cleanup removes in one pass. Must be greater than 0 |
| `PURGE_INTERVAL` | `100` | Milliseconds between cleanup passes. Must be greater than 0 |
| `AOF_PATH` | `/data/appendonly.aof` | Location of the append-only file |
| `AOF_FSYNC` | `everysec` | `always`, `everysec` or `no`. See [Persistence](#persistence-aof) |

The server exits at startup if `PURGE_LIMIT`, `PURGE_INTERVAL` or `AOF_FSYNC` has an invalid value.

## Persistence (AOF)

Every write command that succeeds is appended to the AOF in RESP format. Reads are not logged. On startup the file is replayed to rebuild the data.

**Expiry times.** Relative TTLs are stored as absolute Unix timestamps. For example, `SET k v EX 60` is written as `SET k v PXAT <unix ms>`, and `EXPIRE` is written as `PEXPIREAT`. After a restart a key keeps its original deadline, and keys that expired while the server was down are not restored.

**Damaged files.** If the last record is incomplete (for example after a crash mid-write), it is cut off and a warning is logged. A corrupt record anywhere else makes startup fail, so data is never dropped silently.

**fsync policy.** `AOF_FSYNC` sets how often the file is flushed to disk:

| Value | Behavior |
| --- | --- |
| `always` | Flush after every write. Safest, slowest |
| `everysec` | Flush once per second from a background thread. Up to about one second of writes can be lost on a crash |
| `no` | Leave flushing to the operating system. Fastest |

**Compaction.** `REWRITEAOF` shrinks the file. It writes the current data as the shortest list of commands (`SET`, `RPUSH`, `HSET`, plus `PXAT`/`PEXPIREAT` for keys with a TTL) to a temporary file, then renames it over the old one. If anything fails, the old file stays untouched. Writes are blocked while it runs, and it only runs when `REWRITEAOF` is called. There is no automatic trigger yet.

## Docker

The image is `ghcr.io/shearlingenmouton/minidis`. Available tags are listed on the [releases page](https://github.com/ShearlingEnMouton/minidis/releases). It is based on a distroless image and runs as a non-root user.

The image sets `AOF_PATH=/data/appendonly.aof`, and `/data` is owned by the non-root user. Mounting a volume there keeps the data:

```sh
docker run -d -p 3131:3131 -v minidis-data:/data ghcr.io/shearlingenmouton/minidis:<version>
```

Without a volume, the data is lost when the container is removed.

## Kubernetes (Helm)

The chart is published as an OCI artifact:

```sh
helm install minidis oci://ghcr.io/shearlingenmouton/minidis-helm \
  --version <version> --namespace minidis --create-namespace
```

Main values:

| Value | Default | Description |
| --- | --- | --- |
| `image.repository` | `ghcr.io/shearlingenmouton/minidis` | Image to run |
| `image.tag` | chart `appVersion` | Image tag |
| `logLevel` | `INFO` | Sets `LOG_LEVEL` |
| `purge.limit` | `1000` | Sets `PURGE_LIMIT` |
| `purge.interval` | `100` | Sets `PURGE_INTERVAL` |
| `aof.path` | `/data/appendonly.aof` | Sets `AOF_PATH` |
| `aof.fsync` | `everysec` | Sets `AOF_FSYNC` |
| `pvc.storage` | `2Gi` | Size of the volume that holds the AOF |

The chart creates a `ReadWriteOnce` PersistentVolumeClaim mounted at `/data`. The claim has the `helm.sh/resource-policy: keep` annotation, so `helm uninstall` does not delete the data. The Deployment uses the `Recreate` strategy and runs one replica. minidis is a single-node server and must not be scaled beyond one replica.

## Releases

Pushing a tag like `v0.2.0` or `v0.2.0-rc.1` runs the release workflow, which:

1. Builds the image and pushes it to GHCR, tagged without the leading `v` (`0.2.0`)
2. Packages the Helm chart with the same version and pushes it to GHCR
3. Creates a GitHub Release, marked as a pre-release if the tag contains `-`

## Development

```sh
cargo build
cargo test
```

The repository ships a `pre-push` Git hook that runs `cargo fmt --check` and rejects the push if formatting differs. Enable it once per clone:

```sh
git config core.hooksPath .githooks
```

The CI workflow runs `cargo fmt --check`, `cargo clippy` and `cargo test` on every push to `main` and on pull requests.

| Path | Contents |
| --- | --- |
| `src/main.rs` | Startup, environment settings, client connections |
| `src/resp.rs` | RESP2 reader |
| `src/command.rs`, `src/command/` | Command parsing, execution and replies |
| `src/store.rs` | In-memory store, expiry index, snapshots |
| `src/aof.rs` | AOF append, replay, fsync and rewrite |
| `src/logger/` | Log levels and output |
| `src/limits.rs` | Protocol size limits |
| `helm/minidis/` | Helm chart |
| `.githooks/` | Git hooks |
| `.github/workflows/` | CI and release workflows |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at the licensee's option.

Unless explicitly stated otherwise, any contribution intentionally submitted for inclusion in this project, as defined in the Apache-2.0 license, is dual licensed as above, without any additional terms or conditions.
