# Benchmark

This page records how minidis performs under `redis-benchmark`, compared with Redis on the same machine. The goal was to find out whether the single store lock (`Arc<Mutex<Inner>>`) limits throughput when many clients are connected, before deciding whether to shard it.

Measured on 2026-11-16 at commit `7914339` (0.7.0 development, after the pipelining fix described below). The raw results are in [`benchmarks/2026-11-16.csv`](benchmarks/2026-11-16.csv), and [`benchmarks/run.sh`](benchmarks/run.sh) reproduces the run.

## Setup

| Item | Value |
| --- | --- |
| Machine | Intel Core i5-8500T (6 cores, 6 threads), 14 GiB RAM, NVMe SSD |
| OS | Ubuntu 26.04, Linux 7.0 |
| minidis | Release build (`cargo build --release`), Rust 1.98.1, full build |
| Redis | 7.4.12 (Docker image `redis:7.4`, host network) |
| Client | `redis-benchmark` 7.4 on the same machine, over loopback |

- Both servers keep an AOF on the SSD. minidis ran twice, with `AOF_FSYNC=everysec` and `AOF_FSYNC=no`. Redis ran with `appendonly yes`, `appendfsync everysec` and RDB snapshots off.
- Each case ran `redis-benchmark -t set,get,incr,lpush,hset -n 200000 -r 10000 -c <clients> -P <pipeline>`, with 1, 10, 50 and 200 clients, and a pipeline depth of 1 (one request at a time) and 16.
- The machine also runs a small k3s cluster and a few containers, so the numbers include some background noise. Running the client on the same machine removes network latency, which makes the server the limiting factor.

## Results

Requests per second, minidis with `AOF_FSYNC=everysec` against Redis with `appendfsync everysec`.

**Without pipelining** (`-P 1`):

| Command | Server | 1 client | 10 clients | 50 clients | 200 clients |
| --- | --- | --- | --- | --- | --- |
| `SET` | minidis | 22k | 58k | 57k | 56k |
| `SET` | Redis | 22k | 59k | 58k | 59k |
| `GET` | minidis | 27k | 59k | 58k | 56k |
| `GET` | Redis | 24k | 58k | 58k | 60k |
| `INCR` | minidis | 23k | 58k | 57k | 56k |
| `INCR` | Redis | 22k | 59k | 58k | 59k |
| `LPUSH` | minidis | 24k | 59k | 58k | 56k |
| `LPUSH` | Redis | 22k | 59k | 60k | 59k |
| `HSET` | minidis | 23k | 58k | 58k | 56k |
| `HSET` | Redis | 21k | 59k | 58k | 59k |

**With a pipeline depth of 16** (`-P 16`):

| Command | Server | 1 client | 10 clients | 50 clients | 200 clients |
| --- | --- | --- | --- | --- | --- |
| `SET` | minidis | 179k | 245k | 244k | 126k |
| `SET` | Redis | 247k | 538k | 567k | 528k |
| `GET` | minidis | 366k | 806k | 813k | 766k |
| `GET` | Redis | 322k | 816k | 862k | 758k |
| `INCR` | minidis | 196k | 308k | 300k | 305k |
| `INCR` | Redis | 251k | 585k | 602k | 535k |
| `LPUSH` | minidis | 174k | 298k | 299k | 77k |
| `LPUSH` | Redis | 261k | 551k | 526k | 526k |
| `HSET` | minidis | 192k | 265k | 281k | 279k |
| `HSET` | Redis | 215k | 456k | 513k | 494k |

**200 clients, pipeline depth 16**, with the 99th percentile latency:

| Command | minidis, `everysec` | minidis, `no` | Redis, `everysec` |
| --- | --- | --- | --- |
| `SET` | 126k, p99 89.7 ms | 147k, p99 63.2 ms | 528k, p99 16.7 ms |
| `GET` | 766k, p99 3.1 ms | 752k, p99 7.5 ms | 758k, p99 10.5 ms |
| `INCR` | 305k, p99 23.7 ms | 190k, p99 76.2 ms | 535k, p99 16.2 ms |
| `LPUSH` | 77k, p99 106.4 ms | 240k, p99 46.6 ms | 526k, p99 14.8 ms |
| `HSET` | 279k, p99 32.5 ms | 123k, p99 67.5 ms | 494k, p99 8.4 ms |

## Findings

**The store lock is not a bottleneck.** Without pipelining, minidis and Redis reach the same throughput at every client count, about 58k requests per second from 10 clients up, because each request waits for a network round trip. With pipelining, `GET` reaches about 800k requests per second in both servers up to 200 clients. Reads only take the store lock, so a single lock is enough at this scale. Sharding it is not planned.

**Pipelined writes are limited by the AOF.** With pipelining, write commands reach about 250k to 300k requests per second, roughly half of Redis. Every write command takes the AOF file lock, runs the command, and writes its record to the file with one `write` system call before the next write can start. Redis instead collects the records in memory and writes them once per event loop iteration. The disk is not the limit: the results with `AOF_FSYNC=no` are about the same, and a run with `AOF_PATH=/dev/null` reached 290k `SET`s per second with 50 clients.

**200 pipelining clients increase write latency.** With 200 clients and a pipeline depth of 16, write throughput drops for some commands (for example 126k `SET`s and 77k `LPUSH`es per second with `everysec`) and the 99th percentile latency rises to 20 to 100 ms, against 8 to 17 ms in Redis. The locks are standard library mutexes, so a client task that waits for the AOF lock blocks a whole Tokio worker thread. With more waiting clients than worker threads, the remaining clients have to wait for a thread as well.

A possible improvement is to collect AOF records in a buffer and write them once per batch of commands, as Redis does. It is tracked in `docs/todo.md` and is not needed for typical use.

## Pipelining fix found during the first run

The first run, on `main` at `0e920fd`, stalled at one client with a pipeline depth of 16. minidis wrote each reply with its own `write` call. With pipelining, a batch of 16 requests produced 16 small writes, and Nagle's algorithm held the second and later writes until the client acknowledged the first. The client delays its acknowledgements by up to 40 ms, so every batch waited for that timer.

The fix enables `TCP_NODELAY` on every client connection, as Redis does, and writes the replies to all commands read in one pass with a single `write` call. One client with a pipeline depth of 16, `-n 4000` (before the fix) and `-n 50000` (after):

| Version | `SET` per second | `GET` per second |
| --- | --- | --- |
| Before the fix | 388 | 390 |
| `TCP_NODELAY` only | 62,500 | 95,238 |
| `TCP_NODELAY` and one write per batch | 171,821 | 354,610 |

The integration test `small_pipelined_batches_are_not_delayed` sends 100 batches of 16 commands and fails if they take 2 seconds or more. Before the fix they took about 4 seconds.
