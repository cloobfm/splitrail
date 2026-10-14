#!/usr/bin/env python3
"""Cursor Agent CLI hook that records per-turn token usage for Splitrail (BZL-75).

`cursor-agent` keeps no readable usage locally: transcripts hold text only and the chat store is
encrypted. Its hooks do get usage, undocumented: `afterAgentResponse` and `stop` both receive
the turn's `input_tokens`, `output_tokens`, `cache_read_tokens` and `cache_write_tokens` with the
`model` and `generation_id` (one generation = one user turn; both hooks get the same totals).
`beforeSubmitPrompt` gets the user's `prompt` for the same generation. This script appends one
line per call to `~/.config/splitrail/cursor/usage.jsonl`; the analyzer keeps one record per
generation. Register it in `~/.cursor/hooks.json` for `beforeSubmitPrompt`, `afterAgentResponse`
and `stop`:

    {"version": 1, "hooks": {
      "beforeSubmitPrompt": [{"command": "python3 /path/to/splitrail/scripts/cursor_hook_capture.py"}],
      "afterAgentResponse": [{"command": "python3 /path/to/splitrail/scripts/cursor_hook_capture.py"}],
      "stop": [{"command": "python3 /path/to/splitrail/scripts/cursor_hook_capture.py"}]}}

Prompt and reply are kept as short previews for the dashboard's activity feed, like the Ollama
proxy's. The script must never block or fail the agent: it always lets the prompt through and
exits 0 whatever happens.
"""

import fcntl
import json
import os
import sys
import time

USAGE_FILE = os.path.expanduser("~/.config/splitrail/cursor/usage.jsonl")
# Enough for the activity feed's one-line previews; the transcript is not copied.
PREVIEW_CHARS = 500

TOKEN_KEYS = ("input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens")
# Kept from the payload as-is; prompt and reply become previews, everything else is dropped.
KEEP_KEYS = (
    "hook_event_name",
    "conversation_id",
    "generation_id",
    "session_id",
    "model",
    "status",
    "loop_count",
    "user_email",
    "cursor_version",
) + TOKEN_KEYS


def preview(text):
    if not isinstance(text, str) or not text.strip():
        return None
    text = text.strip()
    return text if len(text) <= PREVIEW_CHARS else text[:PREVIEW_CHARS] + "…"


def record(payload, now=None):
    """The line for a hook payload: a prompt, or a turn's token counts; None otherwise."""
    if not isinstance(payload, dict):
        return None
    prompt = preview(payload.get("prompt"))
    has_tokens = any(isinstance(payload.get(k), (int, float)) for k in TOKEN_KEYS)
    if not prompt and not has_tokens:
        return None
    line = {"ts": time.time() if now is None else now}
    line.update({k: payload[k] for k in KEEP_KEYS if payload.get(k) is not None})
    if prompt:
        line["prompt_preview"] = prompt
    reply = preview(payload.get("text"))
    if reply:
        line["reply_preview"] = reply
    roots = payload.get("workspace_roots")
    if isinstance(roots, list) and roots and isinstance(roots[0], str):
        line["cwd"] = roots[0]
    return line


def append(line):
    os.makedirs(os.path.dirname(USAGE_FILE), exist_ok=True)
    with open(USAGE_FILE, "a") as f:
        # Hooks of parallel agents can fire together; keep lines whole.
        fcntl.flock(f, fcntl.LOCK_EX)
        f.write(json.dumps(line, separators=(",", ":")) + "\n")


def main():
    try:
        line = record(json.load(sys.stdin))
        if line:
            append(line)
    except Exception:  # noqa: BLE001 - a broken capture must not break the agent
        pass
    # beforeSubmitPrompt reads `continue`; the other hooks ignore it.
    print('{"continue": true}')


if __name__ == "__main__":
    main()
