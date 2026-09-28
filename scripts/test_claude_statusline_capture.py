"""Tests for the status line capture's snapshot merging. Run: python3 -m unittest scripts/test_claude_statusline_capture.py"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
from claude_statusline_capture import merge_limits, merge_window  # noqa: E402

WEEK = 1791050400


def w(pct, reset):
    return {"used_percentage": pct, "resets_at": reset}


class MergeTests(unittest.TestCase):
    def test_same_window_keeps_the_higher_usage(self):
        self.assertEqual(merge_window(w(24, WEEK), w(9, WEEK)), w(24, WEEK))
        self.assertEqual(merge_window(w(9, WEEK), w(24, WEEK)), w(24, WEEK))

    def test_new_window_replaces_even_with_lower_usage(self):
        self.assertEqual(merge_window(w(80, WEEK), w(2, WEEK + 7 * 86400)), w(2, WEEK + 7 * 86400))

    def test_stale_session_from_previous_window_is_ignored(self):
        self.assertEqual(merge_window(w(2, WEEK + 7 * 86400), w(80, WEEK)), w(2, WEEK + 7 * 86400))

    def test_missing_window_keeps_what_is_known(self):
        old = {"five_hour": w(3, 1790568000), "seven_day": w(24, WEEK)}
        new = {"seven_day": w(16, WEEK)}
        self.assertEqual(merge_limits(old, new), old)
        self.assertEqual(merge_limits(old, None), old)
        self.assertIsNone(merge_limits(None, None))

    def test_flapping_sequence_converges(self):
        state = None
        for pct in [5, 24, 9, 16, 24, 5]:
            state = merge_limits(state, {"seven_day": w(pct, WEEK)})
        self.assertEqual(state["seven_day"]["used_percentage"], 24)


if __name__ == "__main__":
    unittest.main()
