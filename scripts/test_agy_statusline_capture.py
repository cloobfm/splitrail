"""Tests for the Antigravity status line capture. Run: python3 -m unittest scripts/test_agy_statusline_capture.py"""

import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import agy_statusline_capture as agy  # noqa: E402

NOW = 1790880000.0


def payload(conversation, total_in, total_out, model="gemini-3-pro"):
    return {
        "conversation_id": conversation,
        "session_id": "s1",
        "cwd": "/Users/x/proj",
        "model": {"id": model, "display_name": "Gemini 3 Pro"},
        "context_window": {"total_input_tokens": total_in, "total_output_tokens": total_out},
    }


class QuotaTests(unittest.TestCase):
    def test_buckets_split_by_reset_horizon_keeping_the_fullest(self):
        quota = {
            "gemini-3-pro": {"remaining_fraction": 0.8, "reset_in_seconds": 3 * 3600},
            "gemini-3-flash": {"remaining_fraction": 0.4, "reset_in_seconds": 4 * 3600},
            "weekly": {"remaining_fraction": 0.9, "reset_in_seconds": 5 * 86400},
        }
        windows = agy.quota_windows(quota, NOW)
        self.assertEqual(windows["five_hour"], {"used_percentage": 60.0, "resets_at": NOW + 4 * 3600})
        self.assertEqual(windows["seven_day"]["used_percentage"], 10.0)

    def test_iso_reset_time_wins_over_relative(self):
        quota = {"m": {"remaining_fraction": 0.5, "reset_time": "2026-10-02T00:00:00Z", "reset_in_seconds": 1}}
        self.assertEqual(agy.quota_windows(quota, NOW)["five_hour"]["resets_at"], 1790899200.0)

    def test_missing_or_malformed_quota(self):
        self.assertIsNone(agy.quota_windows(None, NOW))
        self.assertIsNone(agy.quota_windows({"m": {"reset_in_seconds": 5}}, NOW))


class UsageTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.old_dir, agy.USAGE_DIR = agy.USAGE_DIR, self.tmp.name

    def tearDown(self):
        agy.USAGE_DIR = self.old_dir
        self.tmp.cleanup()

    def events(self):
        path = os.path.join(self.tmp.name, "usage.jsonl")
        with open(path) as f:
            return [json.loads(line) for line in f]

    def test_only_growth_is_recorded(self):
        agy.record_usage(payload("c1", 1000, 100), NOW)
        self.assertIsNone(agy.record_usage(payload("c1", 1000, 100), NOW + 1))  # No change.
        agy.record_usage(payload("c1", 1500, 180), NOW + 2)
        agy.record_usage(payload("c2", 50, 5), NOW + 3)
        deltas = [(e["conversation_id"], e["delta_input"], e["delta_output"]) for e in self.events()]
        self.assertEqual(deltas, [("c1", 1000, 100), ("c1", 500, 80), ("c2", 50, 5)])
        self.assertEqual(self.events()[0]["model"], "gemini-3-pro")

    def test_a_restarted_count_is_all_new(self):
        self.assertEqual(agy.usage_delta((1000, 100), (300, 20)), (300, 20))
        self.assertEqual(agy.usage_delta(None, (300, 20)), (300, 20))

    def test_orphaned_temp_files_are_swept(self):
        orphan = os.path.join(self.tmp.name, ".state.abc.tmp")
        open(orphan, "w").close()
        os.utime(orphan, (NOW - 120, NOW - 120))
        agy.record_usage(payload("c1", 10, 1), NOW)
        self.assertFalse(os.path.exists(orphan))

    def test_payload_without_usage_is_skipped(self):
        self.assertIsNone(agy.record_usage({"conversation_id": "c1"}, NOW))
        self.assertIsNone(agy.record_usage({"context_window": {}}, NOW))


if __name__ == "__main__":
    unittest.main()
