"""Tests for cursor-run / agy-run. Run: python3 -m unittest scripts/test_headless_run.py"""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import headless_run  # noqa: E402
from headless_run import TOOLS, agy_usage, cursor_usage, option_value, with_print_flag  # noqa: E402

DATA = os.path.join(os.path.dirname(__file__), "testdata")


def events(name):
    return [json.loads(line) for line in open(os.path.join(DATA, name))]


class CursorTests(unittest.TestCase):
    def test_usage_from_a_real_stream(self):
        line = cursor_usage(events("cursor_stream.jsonl"), ["-p", "hi"], 7.0)
        self.assertEqual(line["ts"], 7.0)
        self.assertEqual(line["hook_event_name"], "print")
        self.assertEqual(line["conversation_id"], "chat-1")
        self.assertEqual(line["generation_id"], "req-1")
        self.assertEqual(line["model"], "Auto")
        self.assertEqual(
            (line["input_tokens"], line["output_tokens"], line["cache_read_tokens"]), (13009, 41, 3840)
        )
        self.assertEqual(line["cwd"], "/Users/x/proj")
        self.assertEqual(line["prompt_preview"], "Reply with just the word ok.")
        self.assertEqual(line["reply_preview"], "ok")

    def test_text_mode_prints_the_result_with_a_newline(self):
        result = TOOLS["cursor-run"]["result"](events("cursor_stream.jsonl"))
        self.assertEqual(TOOLS["cursor-run"]["text"](result), ("ok\n", False))

    def test_explicit_model_wins(self):
        line = cursor_usage(events("cursor_stream.jsonl"), ["--model", "gpt-5"], 1.0)
        self.assertEqual(line["model"], "gpt-5")

    def test_no_result_no_line(self):
        evs = [e for e in events("cursor_stream.jsonl") if e["type"] != "result"]
        self.assertIsNone(cursor_usage(evs, [], 1.0))


class AgyTests(unittest.TestCase):
    def setUp(self):
        self._last = headless_run.last_interactive_agy_model
        headless_run.last_interactive_agy_model = lambda: "Gemini 3.8 Flash (High)"

    def tearDown(self):
        headless_run.last_interactive_agy_model = self._last

    def test_usage_from_a_real_stream(self):
        line = agy_usage(events("agy_stream.jsonl"), ["-p", "hi"], 9.0)
        self.assertEqual(line["conversation_id"], "conv-1")
        self.assertEqual(line["source"], "print")
        self.assertEqual(line["delta_input"], 12808)
        self.assertEqual(line["delta_output"], 28)
        self.assertEqual(line["thinking_tokens"], 27)
        self.assertEqual(line["cwd"], "/Users/x/proj")
        self.assertEqual(line["model"], "Gemini 3.8 Flash (High)")
        self.assertEqual(line["reply_preview"], "ok")

    def test_json_mode_result_is_bare(self):
        bare = events("agy_stream.jsonl")[-1]["result"]
        line = agy_usage([bare], ["--model", "gemini-3-pro"], 1.0)
        self.assertEqual(line["delta_input"], 12808)
        self.assertEqual(line["model"], "gemini-3-pro")

    def test_text_mode_prints_the_response_as_is(self):
        result = TOOLS["agy-run"]["result"](events("agy_stream.jsonl"))
        self.assertEqual(TOOLS["agy-run"]["text"](result), ("ok\n", False))


class PrintFlagTests(unittest.TestCase):
    def test_cursor_gets_a_leading_switch(self):
        self.assertEqual(with_print_flag(TOOLS["cursor-run"], ["--trust", "hi"]), ["-p", "--trust", "hi"])
        self.assertEqual(with_print_flag(TOOLS["cursor-run"], ["--print", "hi"]), ["--print", "hi"])

    def test_agy_binds_the_prompt_to_p(self):
        agy = TOOLS["agy-run"]
        self.assertEqual(
            with_print_flag(agy, ["--output-format", "json", "hi"]),
            ["--output-format", "json", "-p", "hi"],
        )
        self.assertEqual(with_print_flag(agy, ["-p", "hi", "--model", "x"]), ["-p", "hi", "--model", "x"])
        self.assertEqual(with_print_flag(agy, ["--model", "x"]), ["--model", "x", "-p"])


class OptionTests(unittest.TestCase):
    def test_option_parsing(self):
        self.assertEqual(option_value(["--output-format", "json"], "--output-format"), "json")
        self.assertEqual(option_value(["--output-format=stream-json"], "--output-format"), "stream-json")
        self.assertIsNone(option_value(["-p"], "--output-format"))


if __name__ == "__main__":
    unittest.main()
