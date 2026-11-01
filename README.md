# minidis

[![codecov](https://codecov.io/gh/ShearlingEnMouton/minidis/graph/badge.svg)](https://codecov.io/gh/ShearlingEnMouton/minidis)

minidis is a small Redis-compatible server written in Rust. It speaks RESP2 over TCP, so `redis-cli` and most Redis client libraries can connect to it. It is a learning project and implements a limited set of Redis commands, not all of them.

What it does:

- Stores strings, lists and hashes in memory
- Expires keys after a TTL (checked on access, plus a background cleanup)
- Saves writes to an append-only file (AOF) and replays it on startup
- Ships as a Docker image and a Helm chart on GitHub Container Registry (GHCR)
- Comes in two builds: the full build (default) and a minimal lite build. See [Lite build](#lite-build)

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

## Lite build

The lite build is a minimal variant for deployments that only need a small string store. It supports `PING`, `GET`, `SET key value` without options, and `DEL`. Every other command is rejected as an unknown command.

The list, hash and expiry code is removed at compile time, not filtered at runtime. It is behind the Cargo feature `full`, which is on by default. The lite build turns the feature off:

```sh
cargo build --release --no-default-features
```

The lite build keeps AOF persistence, so data survives a restart and a volume is still needed. It has no background cleanup, so `PURGE_LIMIT` and `PURGE_INTERVAL` are ignored. It also has no `REWRITEAOF`, so its AOF is never compacted.

The lite build loads an AOF written by the full build only if every record is a `SET` without expiry or a `DEL`. Otherwise startup fails with an error that names the unsupported command, and the file is left untouched. Switching from the full build to the lite build therefore requires an empty AOF or a new volume. Switching from the lite build to the full build always works.

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
| `PURGE_LIMIT` | `1000` | Most expired keys the background cleanup removes in one pass. Must be greater than 0. Full build only |
| `PURGE_INTERVAL` | `100` | Milliseconds between cleanup passes. Must be greater than 0. Full build only |
| `AOF_PATH` | `/data/appendonly.aof` | Location of the append-only file |
| `AOF_FSYNC` | `everysec` | `always`, `everysec` or `no`. See [Persistence](#persistence-aof) |
| `SHUTDOWN_TIMEOUT` | `10` | Seconds to wait for open client connections when shutting down. Must be greater than 0. See [Shutdown](#shutdown) |

The server exits at startup if `AOF_FSYNC` or `SHUTDOWN_TIMEOUT` has an invalid value, and in the full build also if `PURGE_LIMIT` or `PURGE_INTERVAL` has one.

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

## Shutdown

On `SIGTERM` or `SIGINT` the server shuts down in order:

1. It stops accepting new connections.
2. Idle connections are closed. A command that was already received is executed and answered first.
3. The background cleanup stops.
4. It waits up to `SHUTDOWN_TIMEOUT` seconds for the remaining connections to finish. Connections still open after that are closed without a reply.
5. The AOF is flushed to disk, and the process exits with status 0.

The signal handlers are installed before the AOF is replayed, so a signal received during a long replay is handled once the replay is done.

The server starts listening only after the replay, so clients and the Kubernetes readiness probe cannot connect until the data is loaded.

## Docker

The image is `ghcr.io/shearlingenmouton/minidis`. Available tags are listed on the [releases page](https://github.com/ShearlingEnMouton/minidis/releases). It is based on a distroless image and runs as a non-root user.

The image sets `AOF_PATH=/data/appendonly.aof`, and `/data` is owned by the non-root user. Mounting a volume there keeps the data:

```sh
docker run -d -p 3131:3131 -v minidis-data:/data ghcr.io/shearlingenmouton/minidis:<version>
```

Without a volume, the data is lost when the container is removed.

The lite build is published under the same name with a `-lite` suffix on the tag, for example `ghcr.io/shearlingenmouton/minidis:<version>-lite`. To build it locally, pass the Cargo flag as a build argument:

```sh
docker build --build-arg CARGO_FLAGS=--no-default-features -t minidis:lite .
```

## Kubernetes (Helm)

The chart is published as an OCI artifact:

```sh
helm install minidis oci://ghcr.io/shearlingenmouton/minidis-helm \
  --version <version> --namespace minidis --create-namespace
```

Main values:

| Value | Default | Description |
| --- | --- | --- |
| `image.repository` | `ghcr.io/shearlingenmouton/minidis` | Image to run. Another repository, such as a mirror, must use the same tag scheme: `<version>` for the full build and `<version>-lite` for the lite build |
| `image.tag` | chart `appVersion` | Image tag. When set, it is used as is and `variant` adds no suffix |
| `variant` | `full` | `full` or `lite`. `lite` selects the image tag `<appVersion>-lite`. Any other value makes the chart fail to render |
| `logLevel` | `INFO` | Sets `LOG_LEVEL` |
| `purge.limit` | `1000` | Sets `PURGE_LIMIT`. Not set when `variant` is `lite` |
| `purge.interval` | `100` | Sets `PURGE_INTERVAL`. Not set when `variant` is `lite` |
| `aof.path` | `/data/appendonly.aof` | Sets `AOF_PATH` |
| `aof.fsync` | `everysec` | Sets `AOF_FSYNC`. Must be `always`, `everysec` or `no`, in lowercase. An unquoted `no` in a values file is read by YAML as `false` and is accepted as `no`. Any other value makes the chart fail to render |
| `shutdown.timeout` | `10` | Sets `SHUTDOWN_TIMEOUT` |
| `terminationGracePeriodSeconds` | `30` | Time Kubernetes waits before killing the pod. It is counted from the start of the `preStop` hook (`lifecycle.preStop.sleep.seconds`, 15 by default), so it must be larger than that sleep plus `shutdown.timeout` |
| `startupProbe.periodSeconds` | `5` | Seconds between startup checks |
| `startupProbe.failureThreshold` | `30` | Failed startup checks before the container is restarted. With `periodSeconds`, this gives the server 150 seconds by default to replay the AOF |
| `startupProbe.timeoutSeconds` | `5` | Time limit for one startup check |
| `livenessProbe.periodSeconds` | `5` | Seconds between liveness checks |
| `livenessProbe.failureThreshold` | `3` | Consecutive failed liveness checks before the container is restarted |
| `livenessProbe.timeoutSeconds` | `10` | Time limit for one liveness check |
| `pvc.storage` | `2Gi` | Size of the volume that holds the AOF |

The chart runs minidis as a StatefulSet with one replica. Besides the client Service, it creates a headless Service named `<name>-headless`, which the StatefulSet requires. The StatefulSet creates a `ReadWriteOnce` PersistentVolumeClaim from its claim template and mounts it at `/data`. The claim is named `minidis-data-<name>-sts-0`, which is `minidis-data-minidis-sts-0` by default. Kubernetes does not delete this claim when the StatefulSet is deleted, so `helm uninstall` keeps the data. To remove the data, delete the claim manually.

minidis is a single-node server and must not be scaled beyond one replica. Each additional replica would get its own volume and AOF, and the Service would spread clients across independent datasets.

The claim template cannot be changed on an existing StatefulSet, so changing `pvc.storage` after installation makes `helm upgrade` fail. To resize the volume, edit the PersistentVolumeClaim directly. This works only if its StorageClass allows volume expansion.

**Upgrading from 0.4.x or earlier.** Earlier charts ran a Deployment with a claim named `minidis-pvc`. An upgrade replaces the Deployment with the StatefulSet, which starts with a new, empty claim. The old claim is not deleted because it has the `helm.sh/resource-policy: keep` annotation, but the chart no longer uses it. To keep the data, scale the StatefulSet to zero, copy `appendonly.aof` from the old claim to the new one, then scale it back to one.

**Probes.** The startup and liveness probes run `minidis healthcheck` inside the container. In that mode the binary does not start a server. It connects to the address given by `BIND_ADDR` and `EXPOSE_PORT` (`127.0.0.1` when the server binds to `0.0.0.0`, `::1` for `::`), sends `PING`, and exits with status 0 if the reply is `PONG`. Otherwise it prints the error to stderr and exits with status 1. Kubernetes runs the liveness probe only after the startup probe has succeeded, so a long AOF replay is not mistaken for a stuck server. After that, the liveness probe restarts the container when the server stops answering. The readiness probe checks the TCP port, which opens only after the replay.

Because the claim is kept, changing `variant` from `full` to `lite` on an existing release reuses the old AOF. If that file holds commands the lite build does not support, the pod fails to start until the AOF is emptied or a new volume is used. See [Lite build](#lite-build).

## Releases

Pushing a tag like `v0.3.0` or `v0.3.0-rc.1` runs the release workflow, which:

1. Builds the full and lite images from the same tag and pushes them to GHCR, tagged without the leading `v` (`0.3.0` and `0.3.0-lite`). `latest` is only ever given to the full image
2. Packages the Helm chart with the same version and pushes it to GHCR
3. Creates a GitHub Release, marked as a pre-release if the tag contains `-`

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for the branch and commit conventions and the checks a change must pass.

```sh
cargo build
cargo test
```

The lite build is checked with the same commands and `--no-default-features`:

```sh
cargo clippy --no-default-features --all-targets
cargo test --no-default-features
```

Tests that use lists, hashes or expiry only run in the full build, and tests for the lite behavior only run in the lite build.

Test coverage is measured with [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) over both builds:

```sh
cargo llvm-cov clean --workspace
cargo llvm-cov --no-report
cargo llvm-cov --no-report --no-default-features
cargo llvm-cov report
```

Integration tests stop the server with `SIGTERM`, so the server process exits normally and its coverage data is recorded.

The repository ships a `pre-push` Git hook that runs `cargo fmt --check` and rejects the push if formatting differs. Enable it once per clone:

```sh
git config core.hooksPath .githooks
```

The CI workflow runs `cargo fmt --check`, then `cargo clippy` and `cargo test` for both the full and the lite build, on every push to `main` and on pull requests. The `chart` job runs `helm lint --strict` on the chart and checks that the `aof.fsync` value, given with `--set` or in a values file, reaches the container as `AOF_FSYNC` and that invalid values are rejected. A separate job measures coverage for both builds, shows the summary on the workflow run page, uploads the `lcov.info` report as an artifact, and sends it to [Codecov](https://codecov.io/gh/ShearlingEnMouton/minidis) using GitHub OIDC, so no upload token is stored in the repository. Dependabot checks the Cargo dependencies and the GitHub Actions versions weekly.

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
| `.github/dependabot.yml` | Dependabot configuration |
| `.github/ISSUE_TEMPLATE/`, `.github/pull_request_template.md` | Issue form and pull request template |

## Security

See [SECURITY.md](SECURITY.md) for supported versions and how to report a vulnerability.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at the licensee's option.

Unless explicitly stated otherwise, any contribution intentionally submitted for inclusion in this project, as defined in the Apache-2.0 license, is dual licensed as above, without any additional terms or conditions.
