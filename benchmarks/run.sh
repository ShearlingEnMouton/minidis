#!/bin/bash
# Runs the redis-benchmark suite described in BENCHMARK.md against minidis
# (AOF_FSYNC=everysec and no) and Redis 7.4, and writes one CSV file.
# Usage: benchmarks/run.sh <minidis binary> <work directory> <output csv>
set -u
BIN=$1
DIR=$2
OUT=$3
MINIDIS_PORT=24301
REDIS_PORT=24302
mkdir -p "$DIR/redis"

bench() {
  docker run --rm --network host redis:7.4 redis-benchmark "$@" </dev/null
}

run_suite() {
  for clients in 1 10 50 200; do
    for pipeline in 1 16; do
      bench -p "$2" -t set,get,incr,lpush,hset -n 200000 -c "$clients" -P "$pipeline" -r 10000 -q --csv 2>/dev/null \
        | tail -n +2 | sed "s/^/$1,$clients,$pipeline,/" >>"$OUT"
    done
  done
}

run_minidis() {
  rm -f "$DIR/minidis.aof"
  env BIND_ADDR=127.0.0.1 EXPOSE_PORT=$MINIDIS_PORT AOF_PATH="$DIR/minidis.aof" AOF_FSYNC="$1" \
    LOG_LEVEL=ERR MAX_CLIENTS=10000 IDLE_TIMEOUT=0 "$BIN" >/dev/null 2>&1 &
  local pid=$!
  sleep 1
  run_suite "minidis-$1" $MINIDIS_PORT
  kill $pid
  wait $pid 2>/dev/null
}

echo "label,clients,pipeline,test,rps,avg_ms,min_ms,p50_ms,p95_ms,p99_ms,max_ms" >"$OUT"
run_minidis everysec
run_minidis no

docker run -d --name minidis-bench-redis --network host -v "$DIR/redis:/data" redis:7.4 \
  redis-server --port $REDIS_PORT --bind 127.0.0.1 --appendonly yes --appendfsync everysec --save "" >/dev/null
sleep 2
run_suite redis-everysec $REDIS_PORT
docker rm -f minidis-bench-redis >/dev/null
