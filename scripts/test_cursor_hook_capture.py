"""Tests for the Cursor hook capture. Run: python3 -m unittest scripts/test_cursor_hook_capture.py"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
from cursor_hook_capture import record  # noqa: E402

PAYLOAD = {
    "hook_event_name": "afterAgentResponse",
    "conversation_id": "c1",
    "generation_id": "g1",
    "model": "claude-4.5-sonnet",
    "text": "secret response text",
    "input_tokens": 1200,
    "output_tokens": 300,
    "cache_read_tokens": 50000,
    "cache_write_tokens": 0,
    "workspace_roots": ["/Users/x/proj"],
    "user_email": "me@x.com",
    "transcript_path": "/Users/x/.cursor/projects/p/agent-transcripts/c1/c1.jsonl",
}


class RecordTests(unittest.TestCase):
    def test_keeps_usage_and_a_reply_preview(self):
        line = record(PAYLOAD, now=5.0)
        self.assertEqual(line["ts"], 5.0)
        self.assertEqual(line["output_tokens"], 300)
        self.assertEqual(line["cache_read_tokens"], 50000)
        self.assertEqual(line["cwd"], "/Users/x/proj")
        self.assertEqual(line["generation_id"], "g1")
        self.assertEqual(line["reply_preview"], "secret response text")
        self.assertNotIn("text", line)
        self.assertNotIn("transcript_path", line)

    def test_prompts_are_kept_as_previews(self):
        line = record({"hook_event_name": "beforeSubmitPrompt", "generation_id": "g1",
                       "prompt": "x" * 900, "attachments": []}, now=1.0)
        self.assertEqual(len(line["prompt_preview"]), 501)  # 500 + ellipsis
        self.assertNotIn("attachments", line)

    def test_payloads_without_tokens_are_skipped(self):
        self.assertIsNone(record({"hook_event_name": "sessionEnd", "duration_ms": 10}))
        self.assertIsNone(record({"hook_event_name": "beforeSubmitPrompt", "prompt": "  "}))
        self.assertIsNone(record(None))
        self.assertIsNone(record({**PAYLOAD, **{k: None for k in (
            "input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens")}}))


if __name__ == "__main__":
    unittest.main()
