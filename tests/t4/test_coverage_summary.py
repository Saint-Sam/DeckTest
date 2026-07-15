#!/usr/bin/env python3
"""Focused tests for T4 changed-line coverage accounting."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "coverage_summary", ROOT / "tools/coverage_summary.py"
)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CoverageSummaryTests(unittest.TestCase):
    def test_zero_context_diff_tracks_only_added_side_lines(self) -> None:
        diff = """diff --git a/crates/a.rs b/crates/a.rs
--- a/crates/a.rs
+++ b/crates/a.rs
@@ -2,0 +3,2 @@
+one
+two
@@ -8,1 +10,0 @@
-gone
"""
        self.assertEqual(MODULE.parse_changed_lines(diff), {"crates/a.rs": {3, 4}})

    def test_lcov_is_repository_relative_and_merges_duplicate_counts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "crates/a.rs"
            source.parent.mkdir(parents=True)
            source.write_text("one\ntwo\n", encoding="utf-8")
            lcov = root / "coverage.lcov"
            lcov.write_text(
                f"SF:{source}\nDA:1,0\nDA:1,4\nDA:2,0\nend_of_record\n",
                encoding="utf-8",
            )
            self.assertEqual(
                MODULE.parse_lcov(root, lcov),
                {"crates/a.rs": {1: 4, 2: 0}},
            )


if __name__ == "__main__":
    unittest.main()
