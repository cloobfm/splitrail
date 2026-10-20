"""Tests for the SwiftBar plugin's single-copy rule. Run: python3 -m unittest scripts/test_swiftbar_plugin.py"""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "swiftbar"))
from splitrail import claim, superseded  # noqa: E402


class SingleCopyTests(unittest.TestCase):
    def test_the_newest_copy_runs_and_an_older_one_stops(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "plugin.pid")
            claim(path)
            self.assertFalse(superseded(path, parent=500))
            # A newer copy claims the file: this one is superseded.
            with open(path, "w") as f:
                f.write("999999")
            self.assertTrue(superseded(path, parent=500))

    def test_an_orphaned_copy_stops(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "plugin.pid")
            claim(path)
            self.assertTrue(superseded(path, parent=1))

    def test_no_file_keeps_it_running(self):
        self.assertFalse(superseded("/nonexistent/plugin.pid", pid=1, parent=500))


if __name__ == "__main__":
    unittest.main()
