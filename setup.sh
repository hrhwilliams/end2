#!/bin/sh
# One-time setup for the docker compose testbed: fills in .env, generates the provider's
# signing and VRF keys, and creates the 4-validator CometBFT testnet in config/comet.
# Safe to re-run: it only adds what is missing and never replaces existing keys or chain
# data, since new keys would orphan everything already signed or indexed under the old ones.
set -eu
cd "$(dirname "$0")"
# Stop Git Bash on Windows from rewriting the container paths below.
export MSYS_NO_PATHCONV=1

command -v docker >/dev/null || { echo "setup: docker is required" >&2; exit 1; }

touch .env
# Make sure appended lines start on their own line.
[ -s .env ] && [ -n "$(tail -c 1 .env)" ] && echo >> .env

# get VAR: VAR's value in .env, empty if unset.
get() { sed -n "s/^$1=//p" .env | tail -n 1; }
# put VAR VALUE: sets VAR in .env, replacing any existing line.
put() {
  grep -v "^$1=" .env > .env.tmp || true
  mv .env.tmp .env
  echo "$1=$2" >> .env
}
# default VAR VALUE: sets VAR only when it has no value yet.
default() { [ -n "$(get "$1")" ] || put "$1" "$2"; }

default DB_USER end2
default DB_NAME end2
default DB_PASS "$(od -An -tx1 -N16 /dev/urandom | tr -d ' \n')"
default DB_PORT 5432
default BACKEND_IP 0.0.0.0
# nginx proxies to backend:8080, so this one is fixed.
default BACKEND_PORT 8080

if [ -z "$(get SERVER_SIGNING_KEY)" ] || [ -z "$(get VRF_SECRET_KEY)" ]; then
  echo "setup: building the backend image to generate keys (the first build is slow)"
  docker compose build backend
  keys=$(docker compose run --rm -T --no-deps --entrypoint ./gen_keys backend | tr -d '\r')
  key() { echo "$keys" | sed -n "s/^$1=//p"; }
  # Each public key is only valid next to its own secret, so pairs are written together.
  if [ -z "$(get SERVER_SIGNING_KEY)" ]; then
    put SERVER_SIGNING_KEY "$(key SERVER_SIGNING_KEY)"
    put SERVER_VERIFYING_KEY "$(key SERVER_VERIFYING_KEY)"
    echo "setup: generated SERVER_SIGNING_KEY / SERVER_VERIFYING_KEY"
  fi
  if [ -z "$(get VRF_SECRET_KEY)" ]; then
    put VRF_SECRET_KEY "$(key VRF_SECRET_KEY)"
    put VITE_VRF_PUBLIC_KEY "$(key VITE_VRF_PUBLIC_KEY)"
    echo "setup: generated VRF_SECRET_KEY / VITE_VRF_PUBLIC_KEY"
  fi
fi

if [ ! -f config/comet/node0/config/genesis.json ]; then
  echo "setup: creating the CometBFT testnet in config/comet"
  docker run --rm -v ./config/comet:/cometbft cometbft/cometbft \
    testnet --v 4 --n 0 --o . --populate-persistent-peers --starting-ip-address 192.167.10.2
  # Edit inside the container: on Linux the files above are owned by the container's user.
  docker run --rm -v ./config/comet:/cometbft --entrypoint sh cometbft/cometbft -c '
    for i in 0 1 2 3; do
      sed -i \
        -e "s|proxy_app = \"tcp://127.0.0.1:|proxy_app = \"tcp://abci$i:|" \
        -e "s|laddr = \"tcp://127.0.0.1:|laddr = \"tcp://0.0.0.0:|" \
        /cometbft/node$i/config/config.toml
    done'
fi

echo "setup: done. Start with 'docker compose up --build', then open http://localhost:8000"
