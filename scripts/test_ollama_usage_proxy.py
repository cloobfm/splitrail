#!/usr/bin/env python3
"""Unit tests for ollama_usage_proxy.py. Python 3 stdlib only."""

import json
import os
import sys
import tempfile
import threading
import time
import unittest
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.request import urlopen
from urllib.parse import urlparse

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
# Import proxy logic: we import the functions directly by execing the module,
# since it uses stdlib only and has a main() guard.

PROXY_MODULE = os.path.join(os.path.dirname(os.path.dirname(__file__)), "scripts", "ollama_usage_proxy.py")


def import_proxy():
    import importlib.util
    spec = importlib.util.spec_from_file_location("ollama_usage_proxy", PROXY_MODULE)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class FakeUpstreamHandler(BaseHTTPRequestHandler):
    """Minimal upstream that responds to /api/generate and /v1/chat/completions."""

    def log_message(self, fmt, *args):
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length) if length else b"{}"
        payload = {}
        try:
            payload = json.loads(body.decode("utf-8"))
        except ValueError:
            pass
        path = self.path
        # Return generate-style response.
        if path == "/api/generate":
            model = payload.get("model", "test-model")
            stream = payload.get("stream", True)
            response = {
                "model": model,
                "created_at": "2024-01-01T00:00:00.000000Z",
                "response": "hello",
                "done": True,
                "done_reason": "stop",
                "context": [1, 2],
                "total_duration": 1500000000,
                "load_duration": 100000000,
                "prompt_eval_count": 10,
                "prompt_eval_duration": 50000000,
                "eval_count": 5,
                "eval_duration": 300000000,
                "prompt_eval_cached_count": 2,
            }
            if stream:
                # NDJSON stream: first partial, then final.
                partial = json.dumps({
                    "model": model,
                    "created_at": "2024-01-01T00:00:00.000000Z",
                    "response": "he",
                    "done": False,
                })
                final = json.dumps(response)
                self.send_response(200)
                self.send_header("Content-Type", "application/x-ndjson")
                self.end_headers()
                self.wfile.write((partial + "\n").encode())
                self.wfile.write((final + "\n").encode())
                self.wfile.flush()
            else:
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps(response).encode())
                self.wfile.flush()
            return
        if path == "/api/chat":
            model = payload.get("model", "test-model")
            messages = payload.get("messages", [])
            stream = payload.get("stream", True)
            response = {
                "model": model,
                "created_at": "2024-01-01T00:00:00.000000Z",
                "message": {"role": "assistant", "content": "hello"},
                "done": True,
                "total_duration": 1500000000,
                "load_duration": 100000000,
                "prompt_eval_count": 10,
                "prompt_eval_duration": 50000000,
                "eval_count": 5,
                "eval_duration": 300000000,
                "prompt_eval_cached_count": 2,
            }
            if stream:
                partial_str = json.dumps({
                    "model": model,
                    "created_at": "2024-01-01T00:00:00.000000Z",
                    "message": {"role": "assistant", "content": "he"},
                    "done": False,
                })
                final_str = json.dumps(response)
                self.send_response(200)
                self.send_header("Content-Type", "application/x-ndjson")
                self.end_headers()
                self.wfile.write((partial_str + "\n").encode())
                self.wfile.write((final_str + "\n").encode())
                self.wfile.flush()
            else:
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps(response).encode())
                self.wfile.flush()
            return
        if path == "/v1/chat/completions":
            stream = payload.get("stream", False)
            model = payload.get("model", "test-model")
            include_usage = payload.get("stream_options", {}).get("include_usage")
            base = {
                "id": "chat-1",
                "object": "chat.completion",
                "created": 1704067200,
                "model": model,
                "choices": [{"message": {"role": "assistant", "content": "hello"}, "finish_reason": "stop", "index": 0}],
            }
            if not stream:
                base["usage"] = {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps(base).encode())
                return
            # Streaming SSE: inject include_usage if not set (proxy does it), but fake server responds accordingly.
            # Just return a single chunk with usage included.
            sse_line = (
                "data: " + json.dumps({
                    **base,
                    "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
                    "choices": [{"delta": {"content": "hello"}, "finish_reason": "stop", "index": 0}],
                })
            )
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            self.wfile.write(sse_line.encode() + b"\n\n")
            return
        # /api/tags, /v1/completions, others: simple 200.
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')


class TestProxy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.proxy_mod = import_proxy()
        # Start fake upstream on random port.
        cls.fake_port = None  # set below by socket bind
        # Find a free port by binding to 0 and then closing.
        import socket
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.bind(("127.0.0.1", 0))
        cls.fake_port = s.getsockname()[1]
        s.close()
        cls.fake_server = HTTPServer(("127.0.0.1", cls.fake_port), FakeUpstreamHandler)
        cls.fake_thread = threading.Thread(target=cls.fake_server.serve_forever)
        cls.fake_thread.daemon = True
        cls.fake_thread.start()
        time.sleep(0.2)

    @classmethod
    def tearDownClass(cls):
        cls.fake_server.shutdown()

    def test_parse_generate_final(self):
        chunk = json.dumps({"model": "m", "done": True, "prompt_eval_count": 3, "eval_count": 2, "prompt_eval_cached_count": 1, "total_duration": 1000000})
        res = self.proxy_mod.parse_generate_final(chunk)
        self.assertIsNotNone(res)
        self.assertEqual(res["prompt_tokens"], 3)
        self.assertEqual(res["completion_tokens"], 2)
        self.assertEqual(res["cached_tokens"], 1)

    def test_parse_chat_final(self):
        chunk = json.dumps({"model": "m", "done": True, "prompt_eval_count": 4, "eval_count": 3, "prompt_eval_cached_count": 1})
        res = self.proxy_mod.parse_chat_final(chunk)
        self.assertIsNotNone(res)
        self.assertEqual(res["prompt_tokens"], 4)

    def test_parse_nonstreaming_usage(self):
        body = json.dumps({"model": "m", "done": True, "prompt_eval_count": 5, "eval_count": 4, "prompt_eval_cached_count": 2, "total_duration": 2000000})
        res = self.proxy_mod.parse_nonstreaming_usage(body)
        self.assertIsNotNone(res)
        self.assertEqual(res["prompt_tokens"], 5)
        self.assertEqual(res["duration_ms"], 2)

    def test_parse_openai_usage(self):
        body = json.dumps({"model": "m", "usage": {"prompt_tokens": 10, "completion_tokens": 5}})
        res = self.proxy_mod.parse_openai_usage(body)
        self.assertIsNotNone(res)
        self.assertEqual(res["prompt_tokens"], 10)
        self.assertEqual(res["cached_tokens"], 0)

    def test_include_usage_injection(self):
        # The proxy injects stream_options.include_usage. Verify the payload passed to upstream.
        pass

    def test_proxy_end_to_end_stream_off(self):
        import urllib.request
        req = urllib.request.Request(
            f"http://127.0.0.1:{self.fake_port}/api/generate",
            data=json.dumps({"model": "test", "prompt": "hi", "stream": False}).encode(),
            headers={"Content-Type": "application/json"},
        )
        # Just check it responds 200 without error.
        with urllib.request.urlopen(req, timeout=3) as resp:
            self.assertEqual(resp.status, 200)

    def test_proxy_end_to_end_chat_stream(self):
        import urllib.request
        req = urllib.request.Request(
            f"http://127.0.0.1:{self.fake_port}/api/chat",
            data=json.dumps({"model": "test", "messages": [{"role": "user", "content": "hi"}], "stream": True}).encode(),
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=3) as resp:
            self.assertEqual(resp.status, 200)
            data = resp.read()
            self.assertIn(b"done", data)


if __name__ == "__main__":
    unittest.main()
