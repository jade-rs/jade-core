"""A release comparison must select an immutable baseline and fail closed."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from select_baseline import select_baseline


BASE = "c2c039f289806508c2275e301841035a8fb86fb1"


class BaselineSelectionTests(unittest.TestCase):
    def test_pull_request_uses_its_base(self):
        self.assertEqual(select_baseline("pull_request", BASE, ""), BASE)

    def test_dispatch_can_compare_a_released_commit(self):
        self.assertEqual(select_baseline("workflow_dispatch", "", BASE), BASE)

    def test_dispatch_without_baseline_remains_head_only(self):
        self.assertIsNone(select_baseline("workflow_dispatch", "", ""))

    def test_invalid_or_mutable_baselines_are_rejected(self):
        for value in ("", "main", BASE[:8], "HEAD~1", BASE + "\nrequired=false", "$(echo bad)", "g" * 40):
            with self.subTest(value=value), self.assertRaises(ValueError):
                select_baseline("pull_request", value, "")
        for value in ("main", BASE[:8], BASE + "\n", " " + BASE, "g" * 40):
            with self.subTest(value=value), self.assertRaises(ValueError):
                select_baseline("workflow_dispatch", "", value)

    def test_unsupported_events_are_rejected(self):
        with self.assertRaises(ValueError):
            select_baseline("push", BASE, "")

    def test_cli_outputs_and_failure_do_not_disable_comparison(self):
        script = Path(__file__).with_name("select_baseline.py")
        for event, pr_base, dispatched, expected, succeeds in (
            ("pull_request", BASE, "", f"base_sha={BASE}\nrequired=true\n", True),
            ("workflow_dispatch", "", BASE, f"base_sha={BASE}\nrequired=true\n", True),
            ("workflow_dispatch", "", "", "base_sha=\nrequired=false\n", True),
            ("workflow_dispatch", "", BASE + "\nrequired=false", "", False),
            ("pull_request", "", "", "", False),
        ):
            with self.subTest(event=event, baseline=dispatched), tempfile.TemporaryDirectory() as tmp:
                output = Path(tmp) / "output"
                output.touch()
                result = subprocess.run(
                    [sys.executable, str(script)], capture_output=True, text=True,
                    env={**os.environ, "GITHUB_EVENT_NAME": event, "PR_BASE_SHA": pr_base,
                         "DISPATCH_BASE_SHA": dispatched, "GITHUB_OUTPUT": str(output)},
                )
                self.assertEqual(result.returncode == 0, succeeds, result.stderr)
                self.assertEqual(output.read_text(), expected)


if __name__ == "__main__":
    unittest.main()
