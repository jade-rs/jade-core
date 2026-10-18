#!/usr/bin/env python3
"""Puts the head checkout's perf harness into the base checkout.

    scripts/perf/graft_harness.py <head checkout> <base checkout>

Both sides of a comparison must run the same scenarios, so the base commit is
measured with the head's `crates/jade-docx-perf` rather than its own (which
may differ, or not exist yet). The crate is copied over, and the base
workspace gains the member and the `perf-ci` profile if it lacks them. If the
harness does not compile against the base, the workflow reports the head
alone.
"""

from __future__ import annotations

import re
import shutil
import sys
from pathlib import Path

CRATE = "crates/jade-docx-perf"


def profile_section(manifest: str) -> str | None:
    match = re.search(r"(?ms)^\[profile\.perf-ci\]\n.*?(?=^\[|\Z)", manifest)
    return match.group(0).rstrip() + "\n" if match else None


def main() -> int:
    head, base = (Path(arg) for arg in sys.argv[1:3])
    target = base / CRATE
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(head / CRATE, target)

    manifest_path = base / "Cargo.toml"
    manifest = manifest_path.read_text()
    if f'"{CRATE}"' not in manifest:
        manifest = re.sub(
            r"(?ms)^(members\s*=\s*\[.*?)(^\])",
            lambda m: f'{m.group(1)}  "{CRATE}",\n{m.group(2)}',
            manifest,
            count=1,
        )
    if "[profile.perf-ci]" not in manifest:
        section = profile_section((head / "Cargo.toml").read_text())
        if section is None:
            print("head Cargo.toml has no [profile.perf-ci]", file=sys.stderr)
            return 1
        manifest = manifest.rstrip() + "\n\n" + section
    manifest_path.write_text(manifest)
    return 0


if __name__ == "__main__":
    sys.exit(main())
