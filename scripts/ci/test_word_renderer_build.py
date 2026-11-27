"""Reject unbound or dirty renderer build records using real temporary Git trees."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import record_word_renderer_build as build


class RendererBuildProvenance(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git("init", "--quiet")
        self.git("config", "core.autocrlf", "false")
        self.git("config", "gc.auto", "0")
        self.git("config", "maintenance.auto", "false")
        (self.root / "Cargo.lock").write_bytes(b"original locked dependencies\n")
        (self.root / ".gitignore").write_text("target/\n", encoding="utf-8")
        self.git("add", "Cargo.lock", ".gitignore")
        self.git("-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                 "commit", "--quiet", "-m", "original source")
        self.head = self.git("rev-parse", "HEAD")
        self.exe = self.root / "target" / "probe.exe"
        self.exe.parent.mkdir()
        self.exe.write_bytes(b"test executable bytes")
        self.output = self.root / "target" / "build.json"

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True).strip()

    def record(self, **changes):
        original = subprocess.check_output

        def output(args, **kwargs):
            if args[0] == "rustc":
                return "rustc test\nhost: x86_64-pc-windows-msvc\n"
            if args[0] == "cargo":
                return "cargo test\n"
            return original(args, **kwargs)

        arguments = dict(root=self.root, executable=self.exe, output=self.output,
                         expected_head=self.head)
        arguments.update(changes)
        with patch.object(build.subprocess, "check_output", side_effect=output):
            return build.record(**arguments)

    def test_binds_actual_tree_lock_and_executable_bytes(self):
        record = self.record()
        self.assertEqual(record, json.loads(self.output.read_text(encoding="utf-8")))
        self.assertEqual(record["source_tree"], self.git("rev-parse", "HEAD^{tree}"))
        self.assertEqual(record["head_commit"], self.head)
        self.assertEqual(record["expected_head"], self.head)
        self.assertEqual(record["executable_sha256"], hashlib.sha256(self.exe.read_bytes()).hexdigest())
        self.assertEqual(record["cargo_lock_sha256"], hashlib.sha256((self.root / "Cargo.lock").read_bytes()).hexdigest())
        with self.assertRaisesRegex(ValueError, "replace"):
            self.record()

    def test_dirty_tracked_source_is_rejected_without_publishing(self):
        (self.root / "Cargo.lock").write_bytes(b"changed dependency inputs")
        with self.assertRaises(subprocess.CalledProcessError):
            self.record()
        self.assertFalse(self.output.exists())

    def test_missing_or_unrelated_expected_head_is_rejected(self):
        for head in (None, "f" * 40):
            with self.subTest(head=head), self.assertRaises((ValueError, subprocess.CalledProcessError)):
                self.record(expected_head=head)
            self.assertFalse(self.output.exists())

    def synthetic_merge(self):
        self.git("switch", "--quiet", "-c", "pr-head")
        (self.root / "Cargo.lock").write_bytes(b"PR dependency inputs\n")
        self.git("add", "Cargo.lock")
        self.git("-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                 "commit", "--quiet", "-m", "PR change")
        expected_head = self.git("rev-parse", "HEAD")
        self.git("switch", "--quiet", "--detach", self.head)
        (self.root / "base.txt").write_bytes(b"main branch change\n")
        self.git("add", "base.txt")
        self.git("-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                 "commit", "--quiet", "-m", "main change")
        self.git("-c", "user.name=CI test", "-c", "user.email=ci@example.invalid",
                 "merge", "--quiet", "--no-ff", "pr-head", "-m", "synthetic PR merge")
        return expected_head

    def shallow_checkout(self, depth):
        checkout = self.root / ("checkout-depth-" + str(depth))
        self.git("-c", "advice.detachedHead=false", "clone", "--quiet", "--depth", str(depth),
                 self.root.as_uri(), str(checkout))
        executable = checkout / "target" / "probe.exe"
        executable.parent.mkdir()
        executable.write_bytes(self.exe.read_bytes())
        return dict(root=checkout, executable=executable, output=checkout / "target" / "build.json")

    def test_depth_two_binds_actual_merge_tree_and_pr_parent(self):
        expected_head = self.synthetic_merge()
        record = self.record(expected_head=expected_head, **self.shallow_checkout(2))
        self.assertEqual(record["expected_head"], expected_head)
        self.assertEqual(record["head_commit"], self.git("rev-parse", "HEAD"))
        self.assertEqual(record["source_tree"], self.git("rev-parse", "HEAD^{tree}"))

    def test_depth_one_cannot_claim_an_unavailable_pr_parent(self):
        expected_head = self.synthetic_merge()
        arguments = self.shallow_checkout(1)
        with self.assertRaises(subprocess.CalledProcessError):
            self.record(expected_head=expected_head, **arguments)
        self.assertFalse(arguments["output"].exists())

    def test_paths_cannot_escape_checkout(self):
        for changes in ({"executable": self.root.parent / "outside.exe"},
                        {"output": self.root.parent / "outside.json"}):
            with self.subTest(changes=changes), self.assertRaisesRegex(ValueError, "inside"):
                self.record(**changes)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
