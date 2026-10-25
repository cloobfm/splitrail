#!/bin/sh
# Update a publisher that runs in Docker (the QNAP NAS, ts677) for U (src/self_update.rs), the way
# install_mqtt_publisher.sh updates a systemd one:
#
#   scripts/update_docker_publisher.sh <ssh-host>
#
# The containers run stock images with splitrail's files bind-mounted in: the static binary as
# /usr/local/bin/splitrail-dashboard in the container running `splitrail-dashboard mqtt`, and
# the usage proxy as .../ollama_usage_proxy.py in the proxy's container. This builds the same
# static x86_64 binary, swaps each mounted file (write beside it, then rename, since the running
# one is busy) and restarts its container. The proxy is replaced only when it changed: its
# restart briefly drops the requests in flight to the models behind it.
set -eu
host=$1
cd "$(dirname "$0")/.."

# Container Station's docker is not on a QNAP's PATH.
docker=$(ssh -o BatchMode=yes "$host" 'command -v docker || ls /share/*/.qpkg/container-station/bin/docker 2>/dev/null | head -1')
if [ -z "$docker" ]; then
    echo "$host: no docker found" >&2
    exit 1
fi

# `<container> <host path>` of the file mounted at a destination ending in $1.
mounted() {
    ssh -o BatchMode=yes "$host" "$docker ps --format '{{.Names}}' | while read -r c; do
        $docker inspect \"\$c\" --format '{{range .Mounts}}{{.Destination}} {{.Source}}{{println}}{{end}}' |
            awk -v c=\"\$c\" -v want=\"$1\" 'index(\$1, want) && substr(\$1, length(\$1) - length(want) + 1) == want { print c, \$2 }'
    done | head -1"
}

publisher=$(mounted /usr/local/bin/splitrail-dashboard)
if [ -z "$publisher" ]; then
    echo "$host: no running container mounts /usr/local/bin/splitrail-dashboard" >&2
    exit 1
fi
container=${publisher%% *} source=${publisher#* }

tc=$(dirname "$(rustup which rustc --toolchain nightly)")
PATH="$tc:$HOME/.cargo/bin:$PATH" RUSTC="$tc/rustc" "$tc/cargo" zigbuild --release \
    --target x86_64-unknown-linux-musl --bin splitrail-dashboard --features rusqlite/bundled
bin=target/x86_64-unknown-linux-musl/release/splitrail-dashboard

scp -q -C "$bin" "$host:$source.new"
ssh -o BatchMode=yes "$host" "chmod 755 '$source.new' && mv '$source.new' '$source' && $docker restart '$container' >/dev/null && sleep 2 &&
    $docker inspect -f '{{.State.Status}}' '$container'"

proxy=$(mounted ollama_usage_proxy.py)
if [ -n "$proxy" ]; then
    pcontainer=${proxy%% *} psource=${proxy#* }
    here=$(shasum -a 256 scripts/ollama_usage_proxy.py | cut -d' ' -f1)
    there=$(ssh -o BatchMode=yes "$host" "sha256sum '$psource' 2>/dev/null | cut -d' ' -f1")
    if [ "$here" != "$there" ]; then
        scp -q scripts/ollama_usage_proxy.py "$host:$psource.new"
        ssh -o BatchMode=yes "$host" "mv '$psource.new' '$psource' && $docker restart '$pcontainer' >/dev/null && echo 'proxy updated'"
    fi
fi
