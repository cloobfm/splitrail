#!/bin/sh
# Install `splitrail-dashboard mqtt` on a Linux box as a low-priority systemd user service, so
# its usage rolls up into the dashboards on the same MQTT broker (see src/rollup.rs).
#
#   scripts/install_mqtt_publisher.sh <ssh-host> <broker-host> <mqtt-username> [--share-previews]
#
# Builds a static x86_64 binary here (needs zig and cargo-zigbuild), copies it and a service
# unit over, writes [mqtt] into the box's ~/.splitrail.toml (creating it if missing), and asks
# for the MQTT password without echoing it. Re-running upgrades the binary in place.
#
# It also installs the Claude status line capture (scripts/claude_statusline_capture.py) as the
# box's ~/.claude statusLine, unless one is already set, so the box publishes its account's
# login and plan limits. --share-previews turns on redacted last-message previews.
set -eu
host=$1 broker=$2 user=$3 previews=${4:-}
cd "$(dirname "$0")/.."

tc=$(dirname "$(rustup which rustc --toolchain nightly)")
PATH="$tc:$HOME/.cargo/bin:$PATH" RUSTC="$tc/rustc" "$tc/cargo" zigbuild --release \
    --target x86_64-unknown-linux-musl --bin splitrail-dashboard --features rusqlite/bundled
bin=target/x86_64-unknown-linux-musl/release/splitrail-dashboard

ssh "$host" 'mkdir -p ~/.local/bin ~/.config/splitrail ~/.config/systemd/user'
# Compressed: about 11.5 MB down to 5 MB on the wire; 0.6 s of CPU, minutes saved on a slow link.
scp -q -C "$bin" "$host:.local/bin/splitrail-dashboard.new"
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

if [ "$previews" = --share-previews ]; then
    ssh "$host" "awk '/^\[/{s=(\$0==\"[mqtt]\")} s&&/^share_previews *=/{f=1} END{exit !f}' ~/.splitrail.toml &&
  sed -i '/^\[mqtt\]\$/,/^\[/s/^share_previews *=.*/share_previews = true/' ~/.splitrail.toml ||
  sed -i 's|^\[mqtt\]\$|[mqtt]\nshare_previews = true|' ~/.splitrail.toml"
fi

# Claude plan limits and login only reach Splitrail through the status line.
ssh "$host" 'mkdir -p ~/.local/share/splitrail'
scp -q scripts/claude_statusline_capture.py "$host:.local/share/splitrail/claude_statusline_capture.py"
ssh "$host" 'test -f ~/.claude/settings.json && python3 - <<PY
import json, os, shutil
path = os.path.expanduser("~/.claude/settings.json")
settings = json.load(open(path))
if settings.get("statusLine"):
    print("statusLine already set; left alone")
else:
    shutil.copy(path, path + ".bak-splitrail")
    settings["statusLine"] = {"type": "command", "command": "python3 " + os.path.expanduser("~/.local/share/splitrail/claude_statusline_capture.py") + " main"}
    with open(path + ".tmp", "w") as f:
        json.dump(settings, f, indent=2)
        f.write("\n")
    os.replace(path + ".tmp", path)
    print("statusLine set")
PY'

if ! ssh "$host" 'test -s ~/.config/splitrail/mqtt_password'; then
    printf 'MQTT password for %s: ' "$user"
    stty -echo; read -r password; stty echo; echo
    printf %s "$password" | ssh "$host" 'umask 077; cat > ~/.config/splitrail/mqtt_password'
fi

ssh "$host" 'systemctl --user daemon-reload && systemctl --user enable splitrail-mqtt.service && systemctl --user restart splitrail-mqtt.service && sleep 2 && systemctl --user is-active splitrail-mqtt.service'
