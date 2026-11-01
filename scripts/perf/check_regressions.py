#!/usr/bin/env python3
"""Fail a PR's frozen 18-case Callgrind comparison above +0.5% per case.

Missing or malformed evidence fails closed. Use integer arithmetic for the
boundary so a rounded report cannot turn a regression into a pass.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re


EXPECTED = frozenset(
    f"{scenario}@{fixture}"
    for scenario in ("load", "layout", "repaint", "keystroke", "keystroke-marks", "export")
    for fixture in ("contract.docx", "nda.docx", "yokohama.docx")
)


def instructions(path: Path) -> int:
    lines = path.read_text().splitlines()
    events = [line for line in lines if line.startswith("events:")]
    if events != ["events: Ir"]:
        raise ValueError("expected one instruction-only events declaration")
    counts = []
    for label in ("summary:", "totals:"):
        values = [line for line in lines if line.startswith(label)]
        if len(values) != 1 or not re.fullmatch(re.escape(label) + r" [1-9][0-9]*", values[0]):
            raise ValueError(f"expected one positive integer {label}")
        counts.append(int(values[0].split()[1]))
    if counts[0] != counts[1]:
        raise ValueError("summary and totals disagree")
    return counts[0]


def evaluate(base: Path, head: Path) -> dict:
    errors = []
    for name, folder in (("base", base), ("head", head)):
        found = {p.stem for p in folder.glob("*.out")}
        if found != EXPECTED:
            errors.append(f"{name}: missing={sorted(EXPECTED - found)}, unexpected={sorted(found - EXPECTED)}")
    cases = []
    for case in sorted(EXPECTED):
        try:
            before = instructions(base / (case + ".out"))
            after = instructions(head / (case + ".out"))
        except (OSError, UnicodeError, ValueError) as error:
            errors.append(f"{case}: {error}")
            continue
        passed = (after - before) * 200 <= before
        cases.append({"case": case, "baseIr": before, "headIr": after,
                      "deltaPct": (after - before) * 100 / before, "passed": passed})
        if not passed:
            errors.append(f"{case}: {before} -> {after} instructions exceeds +0.5%")
    return {"thresholdPct": 0.5, "expectedCases": len(EXPECTED),
            "passed": not errors, "cases": cases, "errors": errors}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path)
    parser.add_argument("head", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = evaluate(args.base, args.head)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    for error in result["errors"]:
        print("FAIL: " + error)
    print(f"Callgrind gate: {'PASS' if result['passed'] else 'FAIL'}; "
          f"{len(result['cases'])}/18 comparable cases; per-case limit +0.5%")
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
