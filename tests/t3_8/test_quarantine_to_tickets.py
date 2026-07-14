from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "quarantine_to_tickets", ROOT / "tools/quarantine_to_tickets.py"
)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class QuarantineToTicketsTests(unittest.TestCase):
    def fixture(self, root: Path) -> Path:
        path = root / "quarantine.json"
        path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "source_revision": "abc123",
                    "total_quarantined": 3,
                    "reason_counts": {
                        "NEEDS_NEW_PRIMITIVE": 2,
                        "UNSUPPORTED_VALUE": 1,
                    },
                    "files": [
                        {
                            "path": "z/card.txt",
                            "line": 9,
                            "code": "NEEDS_NEW_PRIMITIVE",
                            "message": "typed vote primitive",
                        },
                        {
                            "path": "a/card.txt",
                            "line": 3,
                            "code": "NEEDS_NEW_PRIMITIVE",
                            "message": "typed vote primitive",
                        },
                        {
                            "path": "b/card.txt",
                            "line": 1,
                            "code": "UNSUPPORTED_VALUE",
                            "message": "not a primitive ticket",
                        },
                    ],
                },
                sort_keys=True,
            ),
            encoding="utf-8",
        )
        return path

    def test_groups_and_sorts_primitive_tickets_deterministically(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            source = self.fixture(Path(temp))
            value = MODULE.load_quarantine(source)
            left = MODULE.build_ticket_queue(value, source)
            right = MODULE.build_ticket_queue(value, source)
            self.assertEqual(left, right)
            self.assertEqual(left["ticket_count"], 1)
            self.assertEqual(left["affected_scripts"], 2)
            self.assertEqual(
                [item["path"] for item in left["tickets"][0]["locations"]],
                ["a/card.txt", "z/card.txt"],
            )
            self.assertEqual(left["tickets"][0]["route"], "T2")

    def test_check_mode_rejects_stale_output(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = self.fixture(root)
            output = root / "tickets.json"
            self.assertEqual(MODULE.main(["--input", str(source), "--output", str(output)]), 0)
            self.assertEqual(
                MODULE.main(["--input", str(source), "--output", str(output), "--check"]),
                0,
            )
            output.write_text("{}\n", encoding="utf-8")
            self.assertEqual(
                MODULE.main(["--input", str(source), "--output", str(output), "--check"]),
                1,
            )

    def test_rejects_duplicate_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            source = self.fixture(Path(temp))
            value = MODULE.load_quarantine(source)
            value["files"][1]["path"] = value["files"][0]["path"]
            with self.assertRaisesRegex(ValueError, "repeats path"):
                MODULE.build_ticket_queue(value, source)


if __name__ == "__main__":
    unittest.main()
