#!/usr/bin/env python3
"""Claude Code status line that records plan-limit usage for Splitrail (BZL-66).

Claude Code runs the `statusLine` command with a JSON payload on stdin and shows whatever it
prints. That payload carries `rate_limits` (5-hour and weekly usage with reset times), which
appears nowhere in the transcripts. This script saves the latest snapshot per account to
`~/.config/splitrail/limits/<label>.json` for the dashboard, and prints a one-line summary.

Configure it once per account, passing that account's label:

    "statusLine": {
      "type": "command",
      "command": "python3 /path/to/splitrail/scripts/claude_statusline_capture.py main"
    }

It never fails the status line: any error still prints something and exits 0.
"""

import json
import os
import sys
import tempfile
import time
from datetime import datetime, timezone

LIMITS_DIR = os.path.expanduser("~/.config/splitrail/limits")

# Windows the payload may carry, in display order, with their short labels.
WINDOWS = [("five_hour", "5h"), ("seven_day", "7d")]


def parse_reset(value):
    """`resets_at` as epoch seconds, from either epoch seconds/ms or an ISO-8601 string."""
    if value is None:
        return None
    if isinstance(value, (int, float)):
        return value / 1000 if value > 1e12 else float(value)
    try:
        return datetime.fromisoformat(str(value).replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def used_percent(window):
    """Usage as 0-100, from `used_percentage` or a 0-1 / 0-100 `utilization`."""
    for key in ("used_percentage", "utilization"):
        value = window.get(key)
        if isinstance(value, (int, float)):
            return value * 100 if key == "utilization" and value <= 1 else float(value)
    return None


def countdown(seconds):
    if seconds <= 0:
        return "now"
    minutes = int(seconds // 60)
    days, minutes = divmod(minutes, 24 * 60)
    hours, minutes = divmod(minutes, 60)
    if days:
        return f"{days}d{hours}h"
    if hours:
        return f"{hours}h{minutes:02d}m"
    return f"{minutes}m"


def save_snapshot(label, payload):
    os.makedirs(LIMITS_DIR, exist_ok=True)
    snapshot = {
        "account": label,
        "captured_at": time.time(),
        "rate_limits": payload.get("rate_limits"),
        # Kept so a payload without rate_limits can be diagnosed from the file alone.
        "payload_keys": sorted(payload.keys()),
    }
    # Write-then-rename: the dashboard may read while a status line refresh is writing.
    fd, tmp = tempfile.mkstemp(dir=LIMITS_DIR, prefix=f".{label}.", suffix=".tmp")
    with os.fdopen(fd, "w") as f:
        json.dump(snapshot, f)
    os.replace(tmp, os.path.join(LIMITS_DIR, f"{label}.json"))


def summary(label, payload):
    parts = [label]
    model = (payload.get("model") or {}).get("display_name")
    if model:
        parts.append(model)
    limits = payload.get("rate_limits") or {}
    now = time.time()
    for key, short in WINDOWS:
        window = limits.get(key)
        if not isinstance(window, dict):
            continue
        pct = used_percent(window)
        reset = parse_reset(window.get("resets_at"))
        text = short
        if pct is not None:
            text += f" {pct:.0f}%"
        if reset is not None:
            text += f" ↻{countdown(reset - now)}"
        parts.append(text)
    return " · ".join(parts)


def main():
    label = sys.argv[1] if len(sys.argv) > 1 else "main"
    try:
        payload = json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError):
        print(label)
        return
    try:
        save_snapshot(label, payload)
    except OSError:
        pass  # A read-only or full disk must not break the status line.
    print(summary(label, payload))


if __name__ == "__main__":
    try:
        main()
    except Exception:  # noqa: BLE001 - the status line must always render
        print(sys.argv[1] if len(sys.argv) > 1 else "main")
