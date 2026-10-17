#!/usr/bin/env python3
"""Ollama usage logging proxy for Splitrail. Python 3 stdlib only.

Ollama has no plugin system and its logs lack output-token counts for MLX models, but every
inference response carries the counts. This proxy sits on Ollama's address, forwards every
request to the real server, streams responses back as they arrive, and appends one line per
inference to `~/.config/splitrail/ollama/usage.jsonl`:

    {"ts": 1790900000.1, "model": "qwen3.8:27b-mlx", "endpoint": "/api/chat",
     "prompt_tokens": 1060, "cached_tokens": 0, "completion_tokens": 212,
     "duration_ms": 8123, "wall_ms": 8140, "client": "127.0.0.1"}

`duration_ms` is Ollama's own `total_duration` (0 for /v1 calls); `wall_ms` is measured here.

Logged: `/api/generate`, `/api/chat` (counts from the final `"done": true` object) and
`/v1/chat/completions`, `/v1/completions` (`usage`; for streams the proxy asks for
`stream_options.include_usage`, so the client also receives the standard usage chunk).
Everything else is passed through untouched. Logging never fails a request.

Configuration (environment):
    OLLAMA_PROXY_HOST   listen address   (default 127.0.0.1)
    OLLAMA_PROXY_PORT   listen port      (default 11434, Ollama's own)
    OLLAMA_UPSTREAM     real Ollama      (default http://127.0.0.1:11435)

Install with `scripts/com.splitrail.ollama-proxy.plist`; steps are in the CHANGELOG.
"""

import http.client
import itertools
import json
import os
import shutil
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse

USAGE_FILE = os.path.expanduser("~/.config/splitrail/ollama/usage.jsonl")

HOP_BY_HOP = {
    "connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te",
    "trailer", "trailers", "transfer-encoding", "upgrade", "proxy-connection",
}
NATIVE = {"/api/generate", "/api/chat"}
OPENAI = {"/v1/chat/completions", "/v1/completions"}
# Decision models (Ollama 0.35+): `usage` is {"input_tokens", "output_tokens"}, with the cached
# prompt count beside it as `prompt_eval_cached_count`.
DECISION = {"/v1/systemone"}
# Largest non-streamed body kept in memory to read its usage; bigger ones pass through unlogged.
MAX_BODY_SCAN = 16 * 1024 * 1024
# Transcript previews for the dashboard's Activity section: the caller's last message and the
# start of the reply, cut to this many characters. Full conversations are never stored.
PREVIEW_CHARS = int(os.environ.get("OLLAMA_PROXY_PREVIEW_CHARS", "500"))


def write_usage(record, path=None):
    """Append one usage line. A single write of one line, so concurrent appends do not tear."""
    path = path or USAGE_FILE
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o644)
        try:
            os.write(fd, (json.dumps(record) + "\n").encode())
        finally:
            os.close(fd)
    except OSError:
        pass


def prepare_request_body(path, body):
    """The body to send upstream: for a streamed OpenAI-style call, ask for the usage chunk."""
    if path not in OPENAI or not body:
        return body
    try:
        payload = json.loads(body)
    except ValueError:
        return body
    if not isinstance(payload, dict) or payload.get("stream") is not True:
        return body
    options = payload.get("stream_options")
    if not isinstance(options, dict):
        options = {}
    if options.get("include_usage") is True:
        return body
    options["include_usage"] = True
    payload["stream_options"] = options
    return json.dumps(payload).encode()


def _text(content):
    """Text of a message `content`: a string, or a list of parts with `text`."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(p.get("text", "") for p in content if isinstance(p, dict))
    return ""


def prompt_preview(path, body):
    """The caller's latest message in a request body, cut to PREVIEW_CHARS."""
    try:
        payload = json.loads(body or b"{}")
    except ValueError:
        return ""
    if not isinstance(payload, dict):
        return ""
    if path in DECISION:
        # `state` (text or JSON) and each question's instructions.
        state = payload.get("state")
        state = state if isinstance(state, str) else json.dumps(state) if state is not None else ""
        questions = payload.get("questions")
        asked = [
            f"{key}: {q.get('instructions')}" if isinstance(q, dict) and isinstance(q.get("instructions"), str) else key
            for key, q in (questions.items() if isinstance(questions, dict) else [])
        ]
        return " · ".join(filter(None, [state.strip(), *asked]))[:PREVIEW_CHARS]
    if path in ("/api/generate", "/v1/completions"):
        prompt = payload.get("prompt")
        text = prompt[-1] if isinstance(prompt, list) and prompt else prompt
        text = text if isinstance(text, str) else ""
    else:
        messages = payload.get("messages")
        users = [m for m in messages if isinstance(m, dict) and m.get("role") == "user"] if isinstance(messages, list) else []
        text = _text(users[-1].get("content")) if users else ""
    return text.strip()[:PREVIEW_CHARS]


def request_model(body):
    """The model a request names, for responses that do not say."""
    try:
        payload = json.loads(body or b"{}")
    except ValueError:
        return ""
    model = payload.get("model") if isinstance(payload, dict) else None
    return model if isinstance(model, str) else ""


def decision_text(answers):
    """A decision model's answers as `key: answer (confidence)`."""
    parts = []
    for key, answer in answers.items():
        if not isinstance(answer, dict):
            continue
        value = next((answer[k] for k in ("choice", "score", "answer", "value") if k in answer), None)
        if value is None:
            value = {k: v for k, v in answer.items() if k not in ("type", "probabilities", "confidence")}
        confidence = answer.get("confidence")
        tail = f" ({confidence:.0%})" if isinstance(confidence, (int, float)) else ""
        parts.append(f"{key}: {value}{tail}")
    return " · ".join(parts)


def reply_text(obj):
    """Reply text carried by one response object, in any of Ollama's dialects."""
    if not isinstance(obj, dict):
        return ""
    if isinstance(obj.get("answers"), dict):
        return decision_text(obj["answers"])  # /v1/systemone
    if isinstance(obj.get("response"), str):
        return obj["response"]  # /api/generate
    message = obj.get("message")
    if isinstance(message, dict):
        return _text(message.get("content"))  # /api/chat
    choices = obj.get("choices")
    if isinstance(choices, list) and choices and isinstance(choices[0], dict):
        choice = choices[0]
        for key in ("delta", "message"):  # /v1/chat/completions, streamed or not
            if isinstance(choice.get(key), dict):
                return _text(choice[key].get("content"))
        if isinstance(choice.get("text"), str):
            return choice["text"]  # /v1/completions
    return ""


def usage_from_object(path, obj):
    """Token counts from one response object, or None if it carries none."""
    if not isinstance(obj, dict):
        return None
    if path in NATIVE:
        if not obj.get("done") or ("eval_count" not in obj and "prompt_eval_count" not in obj):
            return None
        duration = obj.get("total_duration")
        return {
            "model": obj.get("model") or "",
            "prompt_tokens": int(obj.get("prompt_eval_count") or 0),
            "cached_tokens": int(obj.get("prompt_eval_cached_count") or 0),
            "completion_tokens": int(obj.get("eval_count") or 0),
            "duration_ms": int(duration // 1_000_000) if isinstance(duration, int) else 0,
        }
    usage = obj.get("usage")
    if not isinstance(usage, dict):
        return None
    if path in DECISION:
        return {
            "model": obj.get("model") or "",
            "prompt_tokens": int(usage.get("input_tokens") or 0),
            "cached_tokens": int(obj.get("prompt_eval_cached_count") or 0),
            "completion_tokens": int(usage.get("output_tokens") or 0),
            "duration_ms": 0,
        }
    details = usage.get("prompt_tokens_details")
    cached = details.get("cached_tokens") if isinstance(details, dict) else 0
    return {
        "model": obj.get("model") or "",
        "prompt_tokens": int(usage.get("prompt_tokens") or 0),
        "cached_tokens": int(cached or 0),
        "completion_tokens": int(usage.get("completion_tokens") or 0),
        "duration_ms": 0,
    }


class PowerMeter:
    """Measures the energy each request uses, from `macmon` (Apple Silicon, no root needed).

    `macmon pipe` reports SoC power (CPU + GPU + memory + neural engine) every second. Each
    sample's energy is split evenly across the requests running at that moment, so concurrent
    requests share the draw rather than each claiming all of it. Without macmon, `energy_j` is
    simply not recorded and Splitrail falls back to the configured wattage.
    """

    def __init__(self, interval_ms=1000, use_macmon=True):
        self.lock = threading.Lock()
        self.active = {}  # request id -> joules so far
        self.ids = itertools.count()
        self.watts = None  # latest sample
        self.last = None  # monotonic time of the latest sample
        binary = shutil.which("macmon") or next(
            (p for p in ("/opt/homebrew/bin/macmon", "/usr/local/bin/macmon") if os.path.exists(p)), None
        )
        self.available = use_macmon and binary is not None
        if self.available:
            self.proc = subprocess.Popen(
                [binary, "pipe", "-i", str(interval_ms)],
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
            )
            threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            try:
                watts = float(json.loads(line)["all_power"])
            except (ValueError, KeyError, TypeError):
                continue
            self.sample(watts, time.monotonic())
        self.available = False  # macmon exited.

    def sample(self, watts, now):
        with self.lock:
            if self.last is not None and self.active:
                share = (self.watts or watts) * (now - self.last) / len(self.active)
                for key in self.active:
                    self.active[key] += share
            self.watts, self.last = watts, now

    def start(self):
        with self.lock:
            key = next(self.ids)
            self.active[key] = 0.0
            return key

    def stop(self, key):
        """Joules used by a request, including its share since the last sample."""
        with self.lock:
            joules = self.active.pop(key, 0.0)
            if self.watts is not None and self.last is not None:
                joules += self.watts * (time.monotonic() - self.last) / (len(self.active) + 1)
            return joules if self.watts is not None else None


class ModelSizes:
    """Parameter counts (`20.9B`) and base family (`qwen3_5`) per model, from the upstream
    `/api/tags`.

    Splitrail prices Ollama models at the hosted equivalent, which is named by size
    (`gpt-oss-20b`); a `:latest` tag says nothing about size, so the count is logged with usage.
    A custom or fine-tuned model (`tev1:0.8b-mxfp8`) is priced by its family instead.
    Refreshed on a miss, at most once a minute.
    """

    def __init__(self, upstream):
        self.upstream = upstream
        self.sizes = {}
        self.families = {}
        self.fetched = None  # monotonic() can start near 0, so no "long ago" sentinel.
        self.lock = threading.Lock()

    def get(self, model):
        with self.lock:
            stale = self.fetched is None or time.monotonic() - self.fetched > 60
            if model not in self.sizes and stale:
                self.fetched = time.monotonic()
                self._refresh()
            return self.sizes.get(model)

    def family(self, model):
        """The family, once `get` has looked the model up."""
        with self.lock:
            return self.families.get(model)

    def _refresh(self):
        conn = http.client.HTTPConnection(self.upstream.hostname, self.upstream.port or 80, timeout=5)
        try:
            conn.request("GET", "/api/tags")
            for entry in json.loads(conn.getresponse().read()).get("models", []):
                details = entry.get("details") or {}
                size = details.get("parameter_size")
                if entry.get("name") and size:
                    self.sizes[entry["name"]] = size
                if entry.get("name") and details.get("family"):
                    self.families[entry["name"]] = details["family"]
        except (OSError, ValueError, AttributeError):
            pass
        finally:
            conn.close()


class UsageScanner:
    """Finds token counts in a response as it streams past, holding only the current line.

    Handles NDJSON (native streams), SSE `data:` lines (OpenAI-style streams) and a single
    JSON body (non-streamed responses, scanned whole at the end).
    """

    def __init__(self, path, streamed):
        self.path = path
        self.streamed = streamed
        self.partial = b""
        self.body = bytearray()
        self.usage = None
        self.reply = ""

    def feed(self, data):
        if not self.streamed:
            if len(self.body) + len(data) <= MAX_BODY_SCAN:
                self.body += data
            else:
                self.body = None  # Too large to scan; give up on this one.
            return
        lines = (self.partial + data).split(b"\n")
        self.partial = lines.pop()
        for line in lines:
            self._line(line)

    def finish(self):
        if self.streamed:
            if self.partial:
                self._line(self.partial)
        elif self.body:
            self._line(bytes(self.body))
        return self.usage

    def _line(self, line):
        line = line.strip()
        if line.startswith(b"data:"):
            line = line[5:].strip()
        if not line or line == b"[DONE]":
            return
        try:
            obj = json.loads(line)
        except ValueError:
            return
        if len(self.reply) < PREVIEW_CHARS:
            self.reply = (self.reply + reply_text(obj))[:PREVIEW_CHARS]
        found = usage_from_object(self.path, obj)
        if found:
            self.usage = found


class ProxyHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    upstream = urlparse(os.environ.get("OLLAMA_UPSTREAM", "http://127.0.0.1:11435"))
    usage_file = None  # Tests point this elsewhere; None means USAGE_FILE.
    power = None  # PowerMeter, set in main(); None records no energy.
    sizes = None  # ModelSizes, set in main(); None records no parameter counts.

    def log_message(self, fmt, *args):
        pass  # One line per request would flood ~/.splitrail.log.

    def _read_request_body(self):
        if "chunked" in self.headers.get("Transfer-Encoding", "").lower():
            body = bytearray()
            while True:
                size = int(self.rfile.readline().split(b";")[0].strip() or b"0", 16)
                if size == 0:
                    while self.rfile.readline().strip():
                        pass  # Trailers.
                    return bytes(body)
                body += self.rfile.read(size)
                self.rfile.readline()
        length = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(length) if length else b""

    def _proxy(self):
        self.started = time.monotonic()
        path = urlparse(self.path).path
        body = self._read_request_body()
        logged = self.command == "POST" and (path in NATIVE or path in OPENAI or path in DECISION)
        self.prompt = ""
        self.request_model = request_model(body) if logged else ""
        self.power_key = self.power.start() if logged and self.power else None
        if logged:
            self.prompt = prompt_preview(path, body)
            body = prepare_request_body(path, body)

        skip = HOP_BY_HOP | {"host", "content-length"}
        headers = {k: v for k, v in self.headers.items() if k.lower() not in skip}
        headers["Host"] = self.upstream.netloc
        if body or self.command in ("POST", "PUT", "PATCH"):
            headers["Content-Length"] = str(len(body))  # The body may have been rewritten.

        conn = http.client.HTTPConnection(self.upstream.hostname, self.upstream.port or 80)
        try:
            try:
                conn.request(self.command, self.path, body=body or None, headers=headers)
                resp = conn.getresponse()
            except OSError as e:
                self._send_error(502, f"Ollama upstream unreachable: {e}")
                return
            self._relay(resp, path if logged and resp.status == 200 else None)
        finally:
            conn.close()
            if self.power_key is not None:
                self.power.stop(self.power_key)  # No-op if _log already took it.

    def _relay(self, resp, log_path):
        length = resp.getheader("Content-Length")
        has_body = self.command != "HEAD" and resp.status not in (204, 304) and resp.status >= 200
        chunked = has_body and length is None

        self.send_response(resp.status, resp.reason)
        for key, value in resp.getheaders():
            if key.lower() not in HOP_BY_HOP:
                self.send_header(key, value)
        if chunked:
            self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        if not has_body:
            return

        content_type = resp.getheader("Content-Type", "")
        streamed = chunked or "ndjson" in content_type or "event-stream" in content_type
        scanner = UsageScanner(log_path, streamed) if log_path else None
        remaining = None if chunked else int(length)
        try:
            while True:
                data = resp.read1(65536)
                if data and scanner:
                    scanner.feed(data)
                if remaining is not None:
                    remaining -= len(data)
                # Log before the client sees the end of the response, so anything reading the
                # log once a request completes finds its line.
                if scanner and (not data or remaining == 0):
                    self._log(scanner, log_path)
                    scanner = None
                if not data:
                    break
                if chunked:
                    self.wfile.write(b"%x\r\n%s\r\n" % (len(data), data))
                else:
                    self.wfile.write(data)
                self.wfile.flush()
                if remaining == 0:
                    break
            if chunked:
                self.wfile.write(b"0\r\n\r\n")
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            # The client went away; closing the upstream connection stops the generation.
            self.close_connection = True

    def _log(self, scanner, log_path):
        usage = scanner.finish()
        energy = None
        if self.power_key is not None:
            energy = self.power.stop(self.power_key)
            self.power_key = None
        if usage:
            usage["model"] = usage.get("model") or self.request_model
            if energy is not None:
                usage["energy_j"] = round(energy, 2)
            size = self.sizes.get(usage["model"]) if self.sizes and usage.get("model") else None
            if size:
                usage["param_size"] = size
                family = self.sizes.family(usage["model"]) if hasattr(self.sizes, "family") else None
                if family:
                    usage["family"] = family
            write_usage(
                {
                    "ts": time.time(),
                    **usage,
                    # Wall time through the proxy: the only duration for /v1 calls, and the
                    # basis of the electricity cost.
                    "wall_ms": int((time.monotonic() - self.started) * 1000),
                    "endpoint": log_path,
                    "client": self.client_address[0],
                    "prompt_preview": self.prompt,
                    "reply_preview": scanner.reply.strip(),
                },
                self.usage_file,
            )

    def _send_error(self, code, message):
        data = message.encode()
        self.send_response(code)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    do_GET = do_POST = do_PUT = do_DELETE = do_PATCH = do_HEAD = do_OPTIONS = _proxy


class Server(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def handle_error(self, request, client_address):
        # A client dropping an idle keep-alive connection is routine, not worth a traceback.
        if not isinstance(sys.exc_info()[1], (ConnectionResetError, BrokenPipeError, TimeoutError)):
            super().handle_error(request, client_address)


def main():
    host = os.environ.get("OLLAMA_PROXY_HOST", "127.0.0.1")
    port = int(os.environ.get("OLLAMA_PROXY_PORT", "11434"))
    ProxyHandler.power = PowerMeter()
    ProxyHandler.sizes = ModelSizes(ProxyHandler.upstream)
    server = Server((host, port), ProxyHandler)
    upstream = ProxyHandler.upstream.geturl()
    meter = "macmon" if ProxyHandler.power.available else "none (configured watts)"
    print(f"ollama usage proxy {host}:{port} -> {upstream}, logging to {USAGE_FILE}, power: {meter}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    sys.exit(main())
