"""Tests for the Ollama usage proxy. Run: python3 -m unittest scripts/test_ollama_usage_proxy.py

Every test runs against a fake upstream on a random port and logs to a temp dir; nothing here
touches the real Ollama or ~/.config/splitrail.
"""

import http.client
import json
import os
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse

sys.path.insert(0, os.path.dirname(__file__))
import ollama_usage_proxy as proxy  # noqa: E402

FINAL = {
    "model": "qwen3:8b", "done": True, "total_duration": 2_500_000_000,
    "prompt_eval_count": 120, "prompt_eval_cached_count": 20, "eval_count": 30,
}
CHUNK_DELAY = 0.3


class FakeOllama(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    seen_bodies = []

    def log_message(self, *args):
        pass

    def _json(self, obj, status=200):
        data = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _chunks(self, content_type, pieces):
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        for i, piece in enumerate(pieces):
            if i:
                time.sleep(CHUNK_DELAY)
            self.wfile.write(b"%x\r\n%s\r\n" % (len(piece), piece))
            self.wfile.flush()
        self.wfile.write(b"0\r\n\r\n")

    def do_GET(self):
        if self.path == "/api/tags":
            self._json({"models": [{"name": "qwen3:8b", "details": {"parameter_size": "8.2B"}}]})
        else:
            self._json({"error": "not found"}, 404)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])) or b"{}")
        FakeOllama.seen_bodies.append(body)
        path = urlparse(self.path).path
        if path in ("/api/generate", "/api/chat"):
            if body.get("stream", True) is False:
                self._json({**FINAL, "response": "hi"})
            else:
                parts = [{"model": "qwen3:8b", "response": w, "done": False} for w in ("he", "llo")]
                lines = [json.dumps(p).encode() + b"\n" for p in parts + [FINAL]]
                self._chunks("application/x-ndjson", lines)
        elif path == "/v1/chat/completions":
            usage = {"prompt_tokens": 50, "completion_tokens": 7, "total_tokens": 57}
            if body.get("stream"):
                events = [{"model": "qwen3:8b", "choices": [{"delta": {"content": "hi"}}]}]
                if (body.get("stream_options") or {}).get("include_usage"):
                    events.append({"model": "qwen3:8b", "choices": [], "usage": usage})
                sse = [b"data: " + json.dumps(e).encode() + b"\n\n" for e in events]
                self._chunks("text/event-stream", sse + [b"data: [DONE]\n\n"])
            else:
                self._json({"model": "qwen3:8b", "choices": [], "usage": usage})
        elif path == "/api/fail":
            self._json({"error": "boom"}, 500)
        else:
            self._json({"error": "not found"}, 404)


def serve(server):
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


class ProxyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.upstream = serve(ThreadingHTTPServer(("127.0.0.1", 0), FakeOllama))
        cls.tmp = tempfile.TemporaryDirectory()
        cls.usage_file = os.path.join(cls.tmp.name, "usage.jsonl")

        class Handler(proxy.ProxyHandler):
            upstream = urlparse(f"http://127.0.0.1:{cls.upstream.server_port}")
            usage_file = cls.usage_file
            sizes = proxy.ModelSizes(urlparse(f"http://127.0.0.1:{cls.upstream.server_port}"))

        cls.proxy = serve(proxy.Server(("127.0.0.1", 0), Handler))

    @classmethod
    def tearDownClass(cls):
        cls.proxy.shutdown()
        cls.upstream.shutdown()
        cls.tmp.cleanup()

    def setUp(self):
        open(self.usage_file, "w").close()
        FakeOllama.seen_bodies.clear()

    def request(self, method, path, body=None):
        conn = http.client.HTTPConnection("127.0.0.1", self.proxy.server_port, timeout=10)
        data = json.dumps(body).encode() if body is not None else None
        headers = {"Content-Type": "application/json"} if data else {}
        conn.request(method, path, body=data, headers=headers)
        return conn, conn.getresponse()

    def logged(self):
        # The proxy logs before the response ends, so the line is already there.
        with open(self.usage_file) as f:
            return [json.loads(line) for line in f]

    def test_native_stream_arrives_before_it_ends_and_is_logged(self):
        conn, resp = self.request("POST", "/api/generate", {"model": "qwen3:8b", "prompt": "hi"})
        start = time.monotonic()
        first = resp.readline()
        first_at = time.monotonic() - start
        rest = resp.read()
        conn.close()
        self.assertEqual(json.loads(first)["response"], "he")
        self.assertLess(first_at, CHUNK_DELAY, "first chunk was held back until the stream ended")
        self.assertEqual(json.loads(rest.splitlines()[-1]), FINAL)
        [line] = self.logged()
        self.assertEqual(
            {k: line[k] for k in ("model", "endpoint", "prompt_tokens", "cached_tokens", "completion_tokens", "duration_ms", "client")},
            {"model": "qwen3:8b", "endpoint": "/api/generate", "prompt_tokens": 120,
             "cached_tokens": 20, "completion_tokens": 30, "duration_ms": 2500, "client": "127.0.0.1"},
        )
        self.assertEqual((line["prompt_preview"], line["reply_preview"]), ("hi", "hello"))
        self.assertEqual(line["param_size"], "8.2B")
        self.assertGreaterEqual(line["wall_ms"], 2 * CHUNK_DELAY * 1000 - 50)

    def test_native_non_streamed_body_is_returned_whole_and_logged(self):
        conn, resp = self.request("POST", "/api/chat", {"model": "qwen3:8b", "stream": False})
        body = json.loads(resp.read())
        conn.close()
        self.assertEqual(body["response"], "hi")
        [line] = self.logged()
        self.assertEqual((line["endpoint"], line["completion_tokens"]), ("/api/chat", 30))

    def test_openai_stream_gets_usage_injected_and_logged(self):
        messages = [{"role": "user", "content": "first"}, {"role": "assistant", "content": "ok"},
                    {"role": "user", "content": [{"type": "text", "text": "latest"}]}]
        body = {"model": "qwen3:8b", "stream": True, "messages": messages}
        conn, resp = self.request("POST", "/v1/chat/completions", body)
        text = resp.read().decode()
        conn.close()
        self.assertIn("data: [DONE]", text)
        self.assertTrue(FakeOllama.seen_bodies[-1]["stream_options"]["include_usage"])
        [line] = self.logged()
        self.assertEqual((line["prompt_tokens"], line["completion_tokens"]), (50, 7))
        self.assertEqual((line["prompt_preview"], line["reply_preview"]), ("latest", "hi"))

    def test_openai_non_streamed_is_logged_without_rewriting(self):
        body = {"model": "qwen3:8b", "messages": []}
        conn, resp = self.request("POST", "/v1/chat/completions", body)
        resp.read()
        conn.close()
        self.assertNotIn("stream_options", FakeOllama.seen_bodies[-1])
        self.assertEqual(self.logged()[0]["completion_tokens"], 7)

    def test_other_paths_and_errors_pass_through_unlogged(self):
        conn, resp = self.request("GET", "/api/tags")
        self.assertEqual(json.loads(resp.read())["models"][0]["name"], "qwen3:8b")
        conn.close()
        conn, resp = self.request("POST", "/api/fail", {})
        self.assertEqual((resp.status, json.loads(resp.read())), (500, {"error": "boom"}))
        conn.close()
        time.sleep(0.1)
        with open(self.usage_file) as f:
            self.assertEqual(f.read(), "")

    def test_a_long_stream_does_not_block_other_requests(self):
        conn, resp = self.request("POST", "/api/generate", {"model": "qwen3:8b", "prompt": "hi"})
        resp.readline()  # The stream is now open and mid-way.
        start = time.monotonic()
        other, tags = self.request("GET", "/api/tags")
        tags.read()
        self.assertLess(time.monotonic() - start, CHUNK_DELAY)
        other.close()
        resp.read()
        conn.close()

    def test_keep_alive_connection_serves_several_requests(self):
        conn = http.client.HTTPConnection("127.0.0.1", self.proxy.server_port, timeout=10)
        for _ in range(3):
            conn.request("POST", "/api/generate", body=json.dumps({"model": "qwen3:8b", "prompt": "x"}))
            self.assertEqual(json.loads(conn.getresponse().read().splitlines()[-1])["done"], True)
        conn.close()
        self.assertEqual(len(self.logged()), 3)

    def test_unreachable_upstream_is_a_502(self):
        class Dead(proxy.ProxyHandler):
            upstream = urlparse("http://127.0.0.1:1")
            usage_file = self.usage_file

        server = serve(proxy.Server(("127.0.0.1", 0), Dead))
        conn = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=10)
        conn.request("GET", "/api/tags")
        self.assertEqual(conn.getresponse().status, 502)
        conn.close()
        server.shutdown()


class PowerMeterTests(unittest.TestCase):
    def test_energy_is_split_across_concurrent_requests(self):
        meter = proxy.PowerMeter(use_macmon=False)
        meter.sample(30.0, 100.0)
        a = meter.start()
        meter.sample(30.0, 101.0)  # 1 s at 30 W, a alone: 30 J
        b = meter.start()
        meter.sample(30.0, 102.0)  # 1 s at 30 W shared: 15 J each
        meter.last = time.monotonic()  # No partial interval for the stops below.
        self.assertAlmostEqual(meter.stop(a), 45.0, places=1)
        self.assertAlmostEqual(meter.stop(b), 15.0, places=1)

    def test_no_samples_means_no_energy(self):
        meter = proxy.PowerMeter(use_macmon=False)
        self.assertIsNone(meter.stop(meter.start()))


class ParsingTests(unittest.TestCase):
    def test_previews_are_cut_to_length(self):
        long = "x" * (proxy.PREVIEW_CHARS + 50)
        body = json.dumps({"prompt": long}).encode()
        self.assertEqual(len(proxy.prompt_preview("/api/generate", body)), proxy.PREVIEW_CHARS)
        scanner = proxy.UsageScanner("/api/generate", streamed=True)
        scanner.feed((json.dumps({"response": long, "done": False}) + "\n").encode())
        self.assertEqual(len(scanner.reply), proxy.PREVIEW_CHARS)
        self.assertEqual(proxy.prompt_preview("/api/chat", b"not json"), "")

    def test_usage_needs_the_final_native_object(self):
        self.assertIsNone(proxy.usage_from_object("/api/chat", {"done": False, "eval_count": 3}))
        self.assertIsNone(proxy.usage_from_object("/api/chat", {"done": True}))
        self.assertEqual(proxy.usage_from_object("/api/chat", FINAL)["prompt_tokens"], 120)

    def test_openai_cached_tokens_are_read_when_present(self):
        obj = {"usage": {"prompt_tokens": 9, "completion_tokens": 1,
                         "prompt_tokens_details": {"cached_tokens": 4}}}
        self.assertEqual(proxy.usage_from_object("/v1/completions", obj)["cached_tokens"], 4)

    def test_include_usage_is_only_added_to_streams(self):
        add = proxy.prepare_request_body
        self.assertEqual(add("/v1/chat/completions", b'{"stream": false}'), b'{"stream": false}')
        self.assertEqual(add("/api/chat", b'{"stream": true}'), b'{"stream": true}')
        self.assertEqual(add("/v1/completions", b"not json"), b"not json")
        out = json.loads(add("/v1/completions", b'{"stream": true, "stream_options": {"x": 1}}'))
        self.assertEqual(out["stream_options"], {"x": 1, "include_usage": True})

    def test_scanner_handles_lines_split_across_reads(self):
        scanner = proxy.UsageScanner("/api/generate", streamed=True)
        data = b'{"done": false}\n' + json.dumps(FINAL).encode()
        for i in range(0, len(data), 7):
            scanner.feed(data[i:i + 7])
        self.assertEqual(scanner.finish()["completion_tokens"], 30)


if __name__ == "__main__":
    unittest.main()
