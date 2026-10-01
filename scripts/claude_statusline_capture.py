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

Every open session on an account runs its own status line, and each reports the limits it saw
on its own last API call, so an idle session reports old numbers. Overwriting the file with
whatever arrived last made the dashboard flicker (weekly 5% -> 24% -> 9% within a minute).
Snapshots are therefore merged per window: usage within a window only grows, so for the same
window the higher figure wins, a later reset time means a new window, and an earlier reset time
is a stale session and is ignored.
"""

import fcntl
import json
import os
import sys
import tempfile
import time
from datetime import datetime

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


# Reset times of the same window can differ slightly between reports; beyond this they are
# different windows.
SAME_WINDOW_TOLERANCE_SECS = 30 * 60


def merge_window(old, new):
    """The more current of two reports of one window (dicts with used_percentage/resets_at)."""
    if not isinstance(new, dict) or used_percent(new) is None:
        return old
    if not isinstance(old, dict) or used_percent(old) is None:
        return new
    old_reset, new_reset = parse_reset(old.get("resets_at")), parse_reset(new.get("resets_at"))
    if old_reset is not None and new_reset is not None:
        if new_reset > old_reset + SAME_WINDOW_TOLERANCE_SECS:
            return new  # A newer window has started.
        if new_reset < old_reset - SAME_WINDOW_TOLERANCE_SECS:
            return old  # A stale session still reporting the previous window.
    return new if used_percent(new) >= used_percent(old) else old


def merge_limits(old, new):
    old = old if isinstance(old, dict) else {}
    new = new if isinstance(new, dict) else {}
    merged = {}
    for key in set(old) | set(new):
        window = merge_window(old.get(key), new.get(key))
        if window is not None:
            merged[key] = window
    return merged or None


# Temp files older than this were left by a status line process killed mid-write (agy kills
# the previous run when the agent state changes quickly).
STALE_TMP_SECS = 60


def sweep_stale_tmp(directory, now=None):
    now = time.time() if now is None else now
    try:
        names = os.listdir(directory)
    except OSError:
        return
    for name in names:
        if name.startswith(".") and name.endswith(".tmp"):
            path = os.path.join(directory, name)
            try:
                if now - os.path.getmtime(path) > STALE_TMP_SECS:
                    os.remove(path)
            except OSError:
                pass


def save_snapshot(label, payload):
    """Merge this payload into the account's snapshot and return the merged rate limits."""
    os.makedirs(LIMITS_DIR, exist_ok=True)
    path = os.path.join(LIMITS_DIR, f"{label}.json")
    # Sessions refresh concurrently; serialise the read-merge-write so none is lost.
    with open(os.path.join(LIMITS_DIR, f".{label}.lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        sweep_stale_tmp(LIMITS_DIR)
        try:
            with open(path) as f:
                previous = json.load(f).get("rate_limits")
        except (OSError, ValueError, AttributeError):
            previous = None
        merged = merge_limits(previous, payload.get("rate_limits"))
        snapshot = {
            "account": label,
            "captured_at": time.time(),
            "rate_limits": merged,
            # Kept so a payload without rate_limits can be diagnosed from the file alone.
            "payload_keys": sorted(payload.keys()),
        }
        # Write-then-rename: the dashboard may read while a status line refresh is writing.
        fd, tmp = tempfile.mkstemp(dir=LIMITS_DIR, prefix=f".{label}.", suffix=".tmp")
        with os.fdopen(fd, "w") as f:
            json.dump(snapshot, f)
        os.replace(tmp, path)
    return merged


def summary(label, payload, limits):
    parts = [label]
    model = (payload.get("model") or {}).get("display_name")
    if model:
        parts.append(model)
    limits = limits or {}
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
        # Show the merged view, so every session agrees on the account's usage.
        limits = save_snapshot(label, payload)
    except OSError:
        limits = payload.get("rate_limits")  # A read-only or full disk must not break the line.
    print(summary(label, payload, limits))


if __name__ == "__main__":
    try:
        main()
    except Exception:  # noqa: BLE001 - the status line must always render
        print(sys.argv[1] if len(sys.argv) > 1 else "main")
