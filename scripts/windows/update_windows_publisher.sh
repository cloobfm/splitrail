#!/bin/sh
# Build the Windows publisher here and (re)install it on a Windows host over ssh.
#   scripts/windows/update_windows_publisher.sh <ssh host> <publisher name> <broker> <mqtt user>
# Leaves the Ollama proxy task as it is (on or off) and does not restart it.
set -eu
ssh_host=$1 name=$2 broker=$3 user=$4
cd "$(dirname "$0")/../.."
tc=$(dirname "$(rustup which rustc --toolchain nightly)")
PATH="$tc:$HOME/.cargo/bin:$PATH" RUSTC="$tc/rustc" "$tc/cargo" zigbuild --release \
    --target x86_64-pc-windows-gnu --bin splitrail-dashboard --features rusqlite/bundled
ps() { ssh -o BatchMode=yes "$ssh_host" "powershell -NoProfile -EncodedCommand $(printf '%s' "$1" | iconv -t UTF-16LE | base64)"; }
# The running exe is locked: stop it before copying over it.
ps 'New-Item -ItemType Directory -Force -Path $env:USERPROFILE\splitrail | Out-Null; Get-Process splitrail-dashboard -ErrorAction SilentlyContinue | Stop-Process -Force'
scp -q -C target/x86_64-pc-windows-gnu/release/splitrail-dashboard.exe scripts/ollama_usage_proxy.py \
    scripts/windows/ollama_proxy_vrig.pyw scripts/windows/splitrail_publisher.vbs \
    scripts/windows/install_windows_publisher.ps1 "$ssh_host:splitrail/"
ps "\$ProgressPreference='SilentlyContinue'; powershell -NoProfile -ExecutionPolicy Bypass -File \"\$env:USERPROFILE\\splitrail\\install_windows_publisher.ps1\" -Name $name -Broker $broker -User $user"
