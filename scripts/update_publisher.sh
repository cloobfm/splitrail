#!/bin/sh
# Install or update a publisher, Linux or Windows, for U (src/self_update.rs).
#   scripts/update_publisher.sh <ssh host> <publisher name> <broker> <mqtt user>
set -eu
ssh_host=$1 name=$2 broker=$3 user=$4
cd "$(dirname "$0")/.."
os=$(ssh -o BatchMode=yes -o ConnectTimeout=10 "$ssh_host" uname -s 2>/dev/null || true)
if [ "$os" = "Linux" ]; then
    # The Linux installer sets up a systemd service; a host without systemd (the QNAP NAS) runs
    # its publisher in Docker, updated in its container rather than given a second publisher.
    if ! ssh -o BatchMode=yes -o ConnectTimeout=10 "$ssh_host" 'command -v systemctl' >/dev/null 2>&1; then
        exec scripts/update_docker_publisher.sh "$ssh_host"
    fi
    exec scripts/install_mqtt_publisher.sh "$ssh_host" "$broker" "$user"
fi
win=$(ssh -o BatchMode=yes -o ConnectTimeout=10 "$ssh_host" 'echo %OS%' 2>/dev/null | tr -d '\r' || true)
if [ "$win" = "Windows_NT" ]; then
    exec scripts/windows/update_windows_publisher.sh "$ssh_host" "$name" "$broker" "$user"
fi
echo "$ssh_host: neither Linux nor Windows answered over ssh (is [mqtt.ssh] $name set to its ssh host?)" >&2
exit 1
