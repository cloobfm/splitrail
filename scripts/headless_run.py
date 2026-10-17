#!/usr/bin/python3
"""`cursor-run` / `agy-run` / `warp-run`: run Cursor Agent, Antigravity or Warp's agent headless
and record usage (BZL-75).

Both tools are tracked through interactive-only features (Cursor's token hooks, agy's status
line), so a print-mode run (`cursor-agent -p`, `agy -p`, how other agents call them) leaves no
usage behind. Both do report exact usage in the `result` of `--output-format json|stream-json`.
This script is installed under the two command names; the name picks the tool:

    cursor-run [args…] "prompt"   ≡ cursor-agent -p [args…] "prompt"
    agy-run    [args…] "prompt"   ≡ agy -p [args…] "prompt"
    warp-run   [args…]            ≡ warp agent run [args…]

Warp is different: `warp agent run` saves nothing locally (the Warp app's database only has
interactive conversations), and its output has no usage. Its `ndjson` output names the run, and
`warp run get <run id>` (Warp's own sign-in) has the run's credits and cost per model, so
`warp-run` runs as `ndjson`, prints what `text` would, then saves that record to
`~/.config/splitrail/warp/runs/<run id>.json` for the Warp analyzer. `warp-run --record <run id>
[--cwd dir]` saves one after the fact.

* `-p` is added when missing (for agy, whose `-p` takes the prompt as its value, right before
  the prompt, the last argument); everything else goes to the real binary unchanged.
* Caller asked for `json` / `stream-json`: stdout passes through byte for byte.
* Plain text (the default): runs as `stream-json` and prints the final result as text mode does.
* Usage goes to the file the dashboard already reads for that tool, and the raw event stream to
  `~/.config/splitrail/<tool>/runs/<conversation id>.jsonl`.

Recording never changes the caller's output or exit code. The real binaries are the stock
`cursor-agent` / `agy` on PATH, so tool upgrades need nothing from here. Install by linking
`~/.local/bin/cursor-run` and `~/.local/bin/agy-run` to a copy of this file kept outside the repo.
"""

import fcntl
import json
import os
import shutil
import signal
import subprocess
import sys
import threading
import time

CONFIG = os.path.expanduser("~/.config/splitrail")
PREVIEW_CHARS = 500


def preview(text):
    if not isinstance(text, str) or not text.strip():
        return None
    text = text.strip()
    return text if len(text) <= PREVIEW_CHARS else text[:PREVIEW_CHARS] + "…"


def option_value(args, name):
    """Value of `--name value` or `--name=value`, else None."""
    for i, arg in enumerate(args):
        if arg == name and i + 1 < len(args):
            return args[i + 1]
        if arg.startswith(name + "="):
            return arg.split("=", 1)[1]
    return None


# --- Cursor Agent --------------------------------------------------------------------------


def cursor_message_text(message):
    parts = (message or {}).get("content") or []
    return " ".join(p.get("text", "") for p in parts if isinstance(p, dict) and p.get("type") == "text")


def cursor_result(events):
    return next((e for e in reversed(events) if e.get("type") == "result"), None)


def cursor_text(result):
    return (result.get("result") or "") + "\n", bool(result.get("is_error"))


def cursor_usage(events, args, now):
    """A line for `cursor/usage.jsonl` (read by the Cursor Agent analyzer)."""
    result = cursor_result(events)
    if not result or not isinstance(result.get("usage"), dict):
        return None
    init = next((e for e in events if e.get("type") == "system"), {})
    prompt = next((cursor_message_text(e.get("message")) for e in events if e.get("type") == "user"), None)
    usage = result["usage"]
    session = result.get("session_id") or init.get("session_id")
    line = {
        "ts": now,
        "hook_event_name": "print",
        "conversation_id": session,
        "generation_id": result.get("request_id") or f"{session}:{now}",
        "model": option_value(args, "--model") or init.get("model"),
        "status": "error" if result.get("is_error") else result.get("subtype"),
        "input_tokens": usage.get("inputTokens", 0),
        "output_tokens": usage.get("outputTokens", 0),
        "cache_read_tokens": usage.get("cacheReadTokens", 0),
        "cache_write_tokens": usage.get("cacheWriteTokens", 0),
        "duration_ms": result.get("duration_ms"),
        "cwd": init.get("cwd") or os.getcwd(),
        "prompt_preview": preview(prompt),
        "reply_preview": preview(result.get("result")),
    }
    return {k: v for k, v in line.items() if v is not None}


# --- Antigravity -----------------------------------------------------------------------------


def agy_result(events):
    """`stream-json` wraps it as {"event": "result", "result": {…}}; `json` prints it bare."""
    for event in reversed(events):
        if event.get("event") == "result" and isinstance(event.get("result"), dict):
            return event["result"]
        if "usage" in event and "response" in event:
            return event
    return None


def agy_text(result):
    return result.get("response") or "", result.get("status") not in (None, "SUCCESS")


def last_interactive_agy_model():
    """agy's print output names no model; its default is whatever the user last picked, which
    the status line capture records."""
    try:
        with open(os.path.join(CONFIG, "antigravity", "usage.jsonl"), "rb") as f:
            f.seek(0, os.SEEK_END)
            f.seek(max(0, f.tell() - 65536))
            for raw in reversed(f.read().splitlines()):
                try:
                    model = json.loads(raw).get("model")
                except ValueError:
                    continue
                if model:
                    return model
    except OSError:
        pass
    return None


def agy_usage(events, args, now):
    """A line for `antigravity/usage.jsonl` (read by the Antigravity analyzer)."""
    result = agy_result(events)
    if not result or not isinstance(result.get("usage"), dict):
        return None
    init = next((e.get("init") for e in events if e.get("event") == "init"), None) or {}
    usage = result["usage"]
    conversation = result.get("conversation_id") or next(
        (e.get("conversation_id") for e in events if e.get("conversation_id")), None
    )
    line = {
        "ts": now,
        "source": "print",
        "conversation_id": conversation,
        "session_id": conversation,
        "model": option_value(args, "--model") or last_interactive_agy_model(),
        "cwd": init.get("cwd") or os.getcwd(),
        # output_tokens already includes thinking_tokens (input + output == total).
        "delta_input": usage.get("input_tokens", 0),
        "delta_output": usage.get("output_tokens", 0),
        "cache_read_tokens": usage.get("cache_read_tokens", 0),
        "thinking_tokens": usage.get("thinking_tokens", 0),
        "status": result.get("status"),
        "reply_preview": preview(result.get("response")),
    }
    return {k: v for k, v in line.items() if v is not None}


# --- Warp ----------------------------------------------------------------------------------

WARP_DONE = ("SUCCEEDED", "FAILED", "ERROR", "CANCELLED", "BLOCKED")
# What `warp run get` says that the record has no use for (who ran it, links, plan artifacts).
WARP_DROP = ("creator", "executor", "session_link", "artifacts", "children")


def warp_run_id(events):
    return next((e["run_id"] for e in events if isinstance(e.get("run_id"), str)), None)


def warp_text(event):
    """What `--output-format text` would print for one `ndjson` event: the agent's replies."""
    return event.get("text") if event.get("type") == "agent" and isinstance(event.get("text"), str) else None


def warp_record(run, cwd, now):
    """The saved record: the run as `warp run get` reports it, plus where it ran."""
    record = {k: v for k, v in run.items() if k not in WARP_DROP}
    record["cwd"] = cwd
    record["recorded_at"] = now
    return record


def warp_fetch(real, run_id, attempts=6, wait=5.0):
    """`warp run get`, retried until the run is finished with its usage (it lags the exit)."""
    run = None
    for attempt in range(attempts):
        try:
            out = subprocess.run([real, "run", "get", run_id, "--output-format", "json"],
                                 capture_output=True, timeout=60)
            run = json.loads(out.stdout) if out.returncode == 0 else run
        except (OSError, ValueError, subprocess.SubprocessError):
            pass
        if run and run.get("state") in WARP_DONE and run.get("request_usage"):
            return run
        if attempt + 1 < attempts:
            time.sleep(wait)
    return run


def warp_save(real, run_id, cwd, attempts=6):
    try:
        run = warp_fetch(real, run_id, attempts)
        if not run:
            return False
        runs = os.path.join(CONFIG, "warp", "runs")
        os.makedirs(runs, exist_ok=True)
        path = os.path.join(runs, f"{run_id.replace('/', '_')}.json")
        with open(path + ".tmp", "w") as f:
            json.dump(warp_record(run, cwd, time.time()), f)
        os.replace(path + ".tmp", path)
        return True
    except Exception:  # noqa: BLE001 - recording must never affect the run
        return False


def without_option(args, name):
    """`args` minus `--name value` / `--name=value`."""
    out, skip = [], False
    for arg in args:
        if skip:
            skip = False
        elif arg == name:
            skip = True
        elif not arg.startswith(name + "="):
            out.append(arg)
    return out


WARP_PROGRESS_SECS = 30


def warp_progress(real, events, cwd, done):
    """While the run goes on, save its usage so far every `WARP_PROGRESS_SECS`, so the dashboard
    sees it working (and its cost rise) rather than only once it ends."""
    while not done.wait(WARP_PROGRESS_SECS):
        run_id = warp_run_id(events)
        if run_id:
            warp_save(real, run_id, cwd, attempts=1)


def run_warp(real, args):
    if args[:1] == ["--record"] and len(args) >= 2:
        cwd = option_value(args, "--cwd") or option_value(args, "-C") or os.getcwd()
        ok = warp_save(real, args[1], os.path.abspath(cwd))
        sys.stderr.write(f"warp-run: {'saved' if ok else 'could not get'} run {args[1]}\n")
        return 0 if ok else 1
    fmt = option_value(args, "--output-format") or os.environ.get("WARP_OUTPUT_FORMAT") or "pretty"
    cwd = os.path.abspath(option_value(args, "--cwd") or option_value(args, "-C") or os.getcwd())
    child_args = ["agent", "run", *without_option(args, "--output-format"), "--output-format", "ndjson"]
    child = subprocess.Popen([real, *child_args], stdout=subprocess.PIPE)
    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, lambda s, _f: child.send_signal(s))
    out = sys.stdout.buffer
    events, raw_lines = [], []
    done = threading.Event()
    threading.Thread(target=warp_progress, args=(real, events, cwd, done), daemon=True).start()
    for raw in child.stdout:
        raw_lines.append(raw.decode("utf-8", "replace"))
        try:
            event = json.loads(raw)
        except ValueError:
            event = None
        if isinstance(event, dict):
            events.append(event)
        if fmt == "ndjson":
            out.write(raw)
        elif event is None:
            out.write(raw)  # Not an event (an error before the run started): show it as is.
        elif fmt != "json" and warp_text(event):
            out.write(warp_text(event).encode())
        out.flush()
    code = child.wait()
    done.set()
    if fmt == "json":
        out.write(json.dumps(events, indent=2).encode() + b"\n")
        out.flush()
    run_id = warp_run_id(events)
    if run_id:
        try:
            runs = os.path.join(CONFIG, "warp", "runs")
            os.makedirs(runs, exist_ok=True)
            with open(os.path.join(runs, f"{run_id}.events.jsonl"), "w") as f:
                f.writelines(raw_lines)
        except OSError:
            pass
        warp_save(real, run_id, cwd)
    return code


TOOLS = {
    "cursor-run": {
        "binary": "cursor-agent",
        "dir": "cursor",
        "result": cursor_result,
        "text": cursor_text,
        "usage": cursor_usage,
    },
    "agy-run": {
        "binary": "agy",
        "dir": "antigravity",
        "p_takes_prompt": True,
        # From `agy --help` (1.3.5): the options that take a value.
        "value_options": (
            "--add-dir", "--agent", "--conversation", "--effort", "--input-format",
            "--json-schema", "--log-file", "--mode", "--model", "--output-format",
            "--print-timeout", "--project",
        ),
        "result": agy_result,
        "text": agy_text,
        "usage": agy_usage,
    },
    "warp-run": {
        "binary": "warp",
        "run": run_warp,
    },
}


def real_binary(name):
    me = os.path.realpath(__file__)
    for directory in os.environ.get("PATH", "").split(os.pathsep):
        candidate = os.path.join(directory, name)
        if os.access(candidate, os.X_OK) and os.path.realpath(candidate) != me:
            return candidate
    return shutil.which(name)


def record(tool, events, raw_lines, args):
    try:
        line = tool["usage"](events, args, time.time())
        if not line:
            return
        base = os.path.join(CONFIG, tool["dir"])
        runs = os.path.join(base, "runs")
        os.makedirs(runs, exist_ok=True)
        with open(os.path.join(base, "usage.jsonl"), "a") as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            f.write(json.dumps(line, separators=(",", ":")) + "\n")
        name = str(line.get("conversation_id") or int(line["ts"])).replace("/", "_")
        with open(os.path.join(runs, f"{name}.jsonl"), "a") as f:
            f.writelines(raw_lines)
    except Exception:  # noqa: BLE001 - recording must never affect the run
        pass


def with_print_flag(tool, args):
    """`args` with print mode on, if the caller left it off."""
    if any(a in ("-p", "--print") or a.startswith(("-p=", "--print=")) for a in args):
        return list(args)
    if tool.get("p_takes_prompt"):
        # agy: `-p <prompt>`; the prompt is the trailing positional argument, unless that is
        # the value of the option before it (`--model x` with the prompt on stdin).
        value_options = tool.get("value_options", ())
        trailing_is_value = len(args) >= 2 and args[-2] in value_options
        if args and not args[-1].startswith("-") and not trailing_is_value:
            return [*args[:-1], "-p", args[-1]]
        return [*args, "-p"]  # Prompt on stdin.
    return ["-p", *args]


def run(tool, real, args):
    args = with_print_flag(tool, args)
    fmt = option_value(args, "--output-format") or "text"
    child_args = list(args)
    if fmt == "text":
        child_args += ["--output-format", "stream-json"]
    child = subprocess.Popen([real, *child_args], stdout=subprocess.PIPE)
    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, lambda s, _f: child.send_signal(s))

    out = sys.stdout.buffer
    events, raw_lines = [], []
    for raw in child.stdout:
        raw_lines.append(raw.decode("utf-8", "replace"))
        if fmt != "text":
            out.write(raw)  # The caller asked for this format: pass it through as it arrives.
            out.flush()
        try:
            event = json.loads(raw)
            if isinstance(event, dict):
                events.append(event)
        except ValueError:
            pass
    if fmt == "json" and not events and raw_lines:
        # Pretty-printed json spans lines; parse it whole.
        try:
            events = [json.loads("".join(raw_lines))]
        except ValueError:
            pass
    code = child.wait()

    if fmt == "text":
        result = tool["result"](events)
        if result is not None:
            text, is_error = tool["text"](result)
            stream = sys.stderr if is_error else sys.stdout
            stream.write(text)
            stream.flush()
        elif raw_lines:
            # Not stream-json after all (an error before the run started): show it as is.
            sys.stdout.write("".join(raw_lines))
            sys.stdout.flush()
    record(tool, events, raw_lines, args)
    return code


def main():
    name = os.path.basename(sys.argv[0])
    tool = TOOLS.get(name)
    if tool is None:
        sys.stderr.write(f"headless_run: invoke as one of {', '.join(TOOLS)}\n")
        sys.exit(2)
    real = real_binary(tool["binary"])
    if not real:
        sys.stderr.write(f"{name}: {tool['binary']} not found on PATH\n")
        sys.exit(127)
    try:
        code = tool["run"](real, sys.argv[1:]) if "run" in tool else run(tool, real, sys.argv[1:])
    except BrokenPipeError:
        code = 0
    sys.exit(code if code >= 0 else 128 - code)


if __name__ == "__main__":
    main()
