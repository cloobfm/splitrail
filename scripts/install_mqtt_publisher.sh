#!/bin/sh
# Install `splitrail-dashboard mqtt` on a Linux box as a low-priority systemd user service, so
# its usage rolls up into the dashboards on the same MQTT broker (see src/rollup.rs).
#
#   scripts/install_mqtt_publisher.sh <ssh-host> <broker-host> <mqtt-username>
#
# Builds a static x86_64 binary here (needs zig and cargo-zigbuild), copies it and a service
# unit over, writes [mqtt] into the box's ~/.splitrail.toml (creating it if missing), and asks
# for the MQTT password without echoing it. Re-running upgrades the binary in place.
set -eu
host=$1 broker=$2 user=$3
cd "$(dirname "$0")/.."

tc=$(dirname "$(rustup which rustc --toolchain nightly)")
PATH="$tc:$HOME/.cargo/bin:$PATH" RUSTC="$tc/rustc" "$tc/cargo" zigbuild --release \
    --target x86_64-unknown-linux-musl --bin splitrail-dashboard --features rusqlite/bundled
bin=target/x86_64-unknown-linux-musl/release/splitrail-dashboard

ssh "$host" 'mkdir -p ~/.local/bin ~/.config/splitrail ~/.config/systemd/user'
scp -q "$bin" "$host:.local/bin/splitrail-dashboard.new"
ssh "$host" 'mv ~/.local/bin/splitrail-dashboard.new ~/.local/bin/splitrail-dashboard'

ssh "$host" "cat > ~/.config/systemd/user/splitrail-mqtt.service" <<'UNIT'
[Unit]
Description=Splitrail: publish AI coding usage to MQTT
After=network-online.target

[Service]
ExecStart=%h/.local/bin/splitrail-dashboard mqtt
Restart=always
RestartSec=30
Nice=10
IOSchedulingClass=idle
CPUQuota=25%
MemoryMax=1G

[Install]
WantedBy=default.target
UNIT

# `config init` writes an empty [mqtt] section, so look for the broker line, not the section.
ssh "$host" "test -f ~/.splitrail.toml || ~/.local/bin/splitrail-dashboard config init >/dev/null
grep -q '^\[mqtt\]' ~/.splitrail.toml || printf '\n[mqtt]\n' >> ~/.splitrail.toml
awk '/^\[/{s=(\$0==\"[mqtt]\")} s&&/^host *=/{f=1} END{exit !f}' ~/.splitrail.toml ||
  sed -i 's|^\[mqtt\]\$|[mqtt]\nhost = \"$broker\"\nusername = \"$user\"|' ~/.splitrail.toml"

if ! ssh "$host" 'test -s ~/.config/splitrail/mqtt_password'; then
    printf 'MQTT password for %s: ' "$user"
    stty -echo; read -r password; stty echo; echo
    printf %s "$password" | ssh "$host" 'umask 077; cat > ~/.config/splitrail/mqtt_password'
fi

ssh "$host" 'systemctl --user daemon-reload && systemctl --user enable splitrail-mqtt.service && systemctl --user restart splitrail-mqtt.service && sleep 2 && systemctl --user is-active splitrail-mqtt.service'
