#!/usr/bin/env python3
"""Antigravity CLI (`agy`) status line that records token usage and plan quota for Splitrail.

`agy` keeps no readable usage logs, but it runs the `statusLine` command whenever the agent
state changes and pipes it a JSON payload (https://antigravity.google/docs/cli/statusline) with
the conversation's running token totals and the account's per-model quota. This script turns
that into two files the dashboard reads:

- `~/.config/splitrail/antigravity/usage.jsonl`: one line per growth in a conversation's
  totals, `{ts, conversation_id, session_id, model, cwd, delta_input, delta_output}`. Running
  totals per conversation are kept in `state.json` beside it, so each line is only the new part.
- `~/.config/splitrail/limits/antigravity.json`: plan quota in the same snapshot format as the
  Claude Code status line, merged per window by `claude_statusline_capture.merge_limits`.

Configure it in `~/.gemini/antigravity-cli/settings.json`:

    "statusLine": {
      "type": "command",
      "command": "python3 /path/to/splitrail/scripts/agy_statusline_capture.py"
    }

It never fails the status line: any error still prints something and exits 0.
"""

import fcntl
import json
import os
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from claude_statusline_capture import (  # noqa: E402
    WINDOWS,
    countdown,
    parse_reset,
    save_snapshot,
    sweep_stale_tmp,
    used_percent,
)

LABEL = "antigravity"
USAGE_DIR = os.path.expanduser("~/.config/splitrail/antigravity")
# Quota buckets resetting within this horizon fill the 5h row; later ones the weekly row.
SHORT_WINDOW_MAX_SECS = 24 * 3600
# Conversations untouched for this long are dropped from the running-totals state.
STATE_TTL_SECS = 30 * 24 * 3600


def quota_models(quota, now):
    """Each model's quota bucket as `{model: {used_percentage, resets_at}}`, or None."""
    if not isinstance(quota, dict):
        return None
    models = {}
    for model, bucket in quota.items():
        if not isinstance(bucket, dict):
            continue
        remaining = bucket.get("remaining_fraction")
        if not isinstance(remaining, (int, float)):
            continue
        reset = parse_reset(bucket.get("reset_time"))
        if reset is None and isinstance(bucket.get("reset_in_seconds"), (int, float)):
            reset = now + bucket["reset_in_seconds"]
        models[model] = {"used_percentage": round((1 - remaining) * 100, 2), "resets_at": reset}
    return models or None


def quota_windows(quota, now):
    """Per-model quota buckets as `rate_limits`: the emptiest bucket per reset horizon.

    `quota` maps a model id to `{remaining_fraction, reset_time, reset_in_seconds}`. A bucket
    resetting within a day is a short window, any later one a weekly window. Each model has its
    own bucket, and the plan is usable while any model has quota left: one model running out
    (Gemini 4 at 100%) leaves the others, so the plan reads as the model with the most left.
    """
    if not isinstance(quota, dict):
        return None
    windows = {}
    for bucket in quota.values():
        if not isinstance(bucket, dict):
            continue
        remaining = bucket.get("remaining_fraction")
        if not isinstance(remaining, (int, float)):
            continue
        reset = parse_reset(bucket.get("reset_time"))
        if reset is None and isinstance(bucket.get("reset_in_seconds"), (int, float)):
            reset = now + bucket["reset_in_seconds"]
        window = {"used_percentage": round((1 - remaining) * 100, 2), "resets_at": reset}
        key = "five_hour" if reset is None or reset - now <= SHORT_WINDOW_MAX_SECS else "seven_day"
        if key not in windows or used_percent(window) < used_percent(windows[key]):
            windows[key] = window
    return windows or None


def usage_delta(previous, totals):
    """New tokens since the last report of a conversation, as (input, output).

    Totals only grow within a running `agy`; a total below the last one means the conversation
    was resumed in a new process whose count started again from zero, so all of it is new.
    """
    deltas = []
    for old, new in zip(previous or (0, 0), totals):
        deltas.append(new if new < old else new - old)
    return tuple(deltas)


def _write_json(path, data):
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=".state.", suffix=".tmp")
    with os.fdopen(fd, "w") as f:
        json.dump(data, f)
    os.replace(tmp, path)


def record_usage(payload, now=None):
    """Append the conversation's token growth since its last report. Returns the event or None."""
    now = time.time() if now is None else now
    conversation = payload.get("conversation_id") or payload.get("session_id")
    context = payload.get("context_window")
    if not conversation or not isinstance(context, dict):
        return None
    totals = tuple(
        int(context.get(key) or 0) for key in ("total_input_tokens", "total_output_tokens")
    )
    os.makedirs(USAGE_DIR, exist_ok=True)
    state_path = os.path.join(USAGE_DIR, "state.json")
    with open(os.path.join(USAGE_DIR, ".lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        sweep_stale_tmp(USAGE_DIR, now)
        try:
            with open(state_path) as f:
                state = json.load(f)
            if not isinstance(state, dict):
                state = {}
        except (OSError, ValueError):
            state = {}
        seen = state.get(conversation)
        previous = (seen.get("input", 0), seen.get("output", 0)) if isinstance(seen, dict) else None
        delta_input, delta_output = usage_delta(previous, totals)
        state[conversation] = {"input": totals[0], "output": totals[1], "updated": now}
        state = {
            key: value
            for key, value in state.items()
            if isinstance(value, dict) and now - value.get("updated", 0) <= STATE_TTL_SECS
        }
        # State first: a run killed between the two writes then drops one delta rather than
        # counting it again next time.
        _write_json(state_path, state)
        event = None
        if delta_input or delta_output:
            model = payload.get("model") or {}
            event = {
                "ts": now,
                "conversation_id": conversation,
                "session_id": payload.get("session_id"),
                "model": model.get("id") or model.get("display_name"),
                "cwd": payload.get("cwd") or (payload.get("workspace") or {}).get("current_dir"),
                "delta_input": delta_input,
                "delta_output": delta_output,
            }
            with open(os.path.join(USAGE_DIR, "usage.jsonl"), "a") as f:
                f.write(json.dumps(event) + "\n")
    return event


def summary(payload, limits):
    parts = []
    model = (payload.get("model") or {}).get("display_name")
    if model:
        parts.append(model)
    context = payload.get("context_window")
    if isinstance(context, dict) and isinstance(context.get("used_percentage"), (int, float)):
        parts.append(f"ctx {context['used_percentage']:.0f}%")
    now = time.time()
    for key, short in WINDOWS:
        window = (limits or {}).get(key)
        if not isinstance(window, dict):
            continue
        text = short
        pct = used_percent(window)
        if pct is not None:
            text += f" {pct:.0f}%"
        reset = parse_reset(window.get("resets_at"))
        if reset is not None:
            text += f" ↻{countdown(reset - now)}"
        parts.append(text)
    return " · ".join(parts) or "agy"


def main():
    try:
        payload = json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError):
        print("agy")
        return
    if not isinstance(payload, dict):
        print("agy")
        return
    try:
        record_usage(payload)
    except OSError:
        pass  # A read-only or full disk must not break the line.
    windows = quota_windows(payload.get("quota"), time.time())
    models = quota_models(payload.get("quota"), time.time())
    try:
        # The raw buckets, to check how they map to windows (agy does not document them), and
        # each model's quota. Only when this report had them: most reports carry no quota.
        if payload.get("quota") is not None:
            os.makedirs(USAGE_DIR, exist_ok=True)
            _write_json(
                os.path.join(USAGE_DIR, "quota_raw.json"),
                {"captured_at": time.time(), "quota": payload.get("quota"),
                 "plan_tier": payload.get("plan_tier")},
            )
        if models:
            _write_json(os.path.join(USAGE_DIR, "quota_models.json"), {"captured_at": time.time(), "models": models})
    except OSError:
        pass
    try:
        # Snapshot fields other than the quota are not kept; the key list aids diagnosis.
        limits = save_snapshot(
            LABEL, {**payload, "rate_limits": windows}, login=payload.get("email") or LABEL
        )["rate_limits"]
    except OSError:
        limits = windows
    print(summary(payload, limits))


if __name__ == "__main__":
    try:
        main()
    except Exception:  # noqa: BLE001 - the status line must always render
        print("agy")
