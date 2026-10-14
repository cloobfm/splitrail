"""Tests for the status line capture's snapshot merging. Run: python3 -m unittest scripts/test_claude_statusline_capture.py"""

import os
import sys
import time
import unittest

sys.path.insert(0, os.path.dirname(__file__))
from claude_statusline_capture import (  # noqa: E402
    claude_json_path,
    login_from_account,
    merge_limits,
    merge_window,
    summary,
    update_snapshot,
)

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


NOW = WEEK - 3 * 86400


def report(session, pct, reset=WEEK):
    return {"session_id": session, "rate_limits": {"seven_day": w(pct, reset)}}


class LoginTests(unittest.TestCase):
    def test_limits_are_kept_per_login(self):
        state = update_snapshot(None, "v", report("s1", 50), "a@x.com", NOW)
        state = update_snapshot(state, "v", report("s1", 10, WEEK + 3600 * 5), "b@x.com", NOW + 60)
        self.assertEqual(state["login"], "b@x.com")
        self.assertEqual(state["rate_limits"]["seven_day"]["used_percentage"], 10)
        self.assertEqual(state["logins"]["a@x.com"]["rate_limits"]["seven_day"]["used_percentage"], 50)
        # Switching back shows the first login's own figure, not the other's.
        state = update_snapshot(state, "v", report("s1", 52), "a@x.com", NOW + 120)
        self.assertEqual(state["rate_limits"]["seven_day"]["used_percentage"], 52)
        self.assertEqual(state["logins"]["b@x.com"]["rate_limits"]["seven_day"]["used_percentage"], 10)

    def test_idle_session_after_a_switch_is_not_attributed_to_the_new_login(self):
        state = update_snapshot(None, "v", report("idle", 80), "a@x.com", NOW)
        # After /login to b, the idle session still repeats a's numbers.
        state = update_snapshot(state, "v", report("idle", 80), "b@x.com", NOW + 60)
        self.assertIsNone(state["rate_limits"])
        self.assertNotIn("b@x.com", state["logins"])
        # A session that made a call under b reports b's numbers.
        state = update_snapshot(state, "v", report("busy", 5, WEEK + 7200), "b@x.com", NOW + 90)
        self.assertEqual(state["rate_limits"]["seven_day"]["used_percentage"], 5)
        self.assertEqual(state["logins"]["a@x.com"]["rate_limits"]["seven_day"]["used_percentage"], 80)

    def test_unchanged_usage_under_the_same_login_still_fills_an_empty_login(self):
        # Sessions recorded before logins were tracked, then a key change left the login empty.
        state = {"sessions": {"s1": {"fp": '{"seven_day": {"resets_at": %d, "used_percentage": 23}}' % WEEK,
                                     "seen": NOW}}}
        state = update_snapshot(state, "main", report("s1", 23), "me (personal)", NOW + 60)
        self.assertEqual(state["rate_limits"]["seven_day"]["used_percentage"], 23)
        state = update_snapshot(state, "main", report("s1", 23), "me (personal)", NOW + 120)
        self.assertEqual(state["rate_limits"]["seven_day"]["used_percentage"], 23)

    def test_a_session_that_just_called_replaces_a_wrong_figure(self):
        # A wrong 100% for b (another login's, written during a switch) in the current window.
        state = update_snapshot(None, "v", report("old", 100), "b@x.com", NOW)
        self.assertEqual(state["rate_limits"]["seven_day"]["used_percentage"], 100)
        # A repeat does not lower it: sessions repeat whatever they last saw.
        state = update_snapshot(state, "v", report("s1", 20), "b@x.com", NOW + 30)
        state = update_snapshot(state, "v", report("old", 100), "b@x.com", NOW + 60)
        self.assertEqual(state["rate_limits"]["seven_day"]["used_percentage"], 20)
        # s1's figure changed: it just called the API, so its number is the current one.
        state = update_snapshot(state, "v", report("s1", 25), "b@x.com", NOW + 90)
        self.assertEqual(state["rate_limits"]["seven_day"]["used_percentage"], 25)
        state = update_snapshot(state, "v", report("old", 100), "b@x.com", NOW + 120)
        self.assertEqual(state["rate_limits"]["seven_day"]["used_percentage"], 25)

    def test_a_new_session_echoing_another_login_is_not_live(self):
        state = update_snapshot(None, "v", report("a1", 90), "a@x.com", NOW)
        state = update_snapshot(state, "v", report("b1", 10), "b@x.com", NOW + 30)
        state = update_snapshot(state, "v", report("b2", 90), "b@x.com", NOW + 60)
        # b2's first report matches a's numbers exactly: merged, not trusted outright.
        self.assertEqual(state["logins"]["b@x.com"]["rate_limits"]["seven_day"]["used_percentage"], 90)
        state = update_snapshot(state, "v", report("b1", 12), "b@x.com", NOW + 90)
        self.assertEqual(state["rate_limits"]["seven_day"]["used_percentage"], 12)

    def test_old_sessions_are_forgotten(self):
        state = update_snapshot(None, "v", report("s1", 5), "a@x.com", NOW)
        state = update_snapshot(state, "v", report("s2", 6), "a@x.com", NOW + 2 * 86400)
        self.assertEqual(set(state["sessions"]), {"s2"})

    def test_summary_lists_other_logins_weekly_usage(self):
        future = time.time() + 86400  # summary() counts down from the real clock
        state = update_snapshot(None, "v", report("s1", 50, future), "a@x.com", NOW)
        state = update_snapshot(state, "v", report("s2", 10, future), "b@x.com", NOW)
        line = summary("v", {}, state["rate_limits"], state)
        self.assertTrue(line.startswith("v:b · "))
        self.assertTrue(line.endswith("7d a 50%"))

    def test_personal_and_team_logins_of_one_email_are_distinct(self):
        personal = {"emailAddress": "me@x.com", "organizationType": "claude_max",
                    "organizationName": "me@x.com's Organization"}
        team = {"emailAddress": "me@x.com", "organizationType": "claude_team",
                "organizationName": "Acme"}
        self.assertEqual(login_from_account(personal), ("me@x.com (personal)", "me"))
        self.assertEqual(login_from_account(team), ("me@x.com (Acme)", "me·team"))
        self.assertEqual(login_from_account({}), (None, None))

    def test_config_dir_comes_from_env_then_label(self):
        home = os.path.dirname(__file__)
        self.assertEqual(claude_json_path("v", {"CLAUDE_CONFIG_DIR": "/c"}, home), "/c/.claude.json")
        self.assertEqual(claude_json_path("main", {}, "/h"), "/h/.claude.json")


if __name__ == "__main__":
    unittest.main()
