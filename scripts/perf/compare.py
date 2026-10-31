#!/usr/bin/env python3
"""Turns two directories of callgrind runs into a pull-request comment.

    scripts/perf/compare.py <base dir> <head dir> [--base-label X] [--head-label Y]

Each directory holds `<scenario>@<fixture>.out` files from
`scripts/perf/run-callgrind.sh`. The report gives each scenario's instruction
count on both sides and, for every scenario that moved, the functions whose
own (exclusive) instructions moved most. Instruction counts are close to
deterministic, so a difference of a fraction of a percent is real; the
threshold below only keeps hash-seed noise out of the per-function tables.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

HEADER = "### ⚙️ jade-core perf (callgrind)"
# Scenarios that moved less than this are reported but not broken down.
BREAKDOWN_THRESHOLD_PCT = 0.5
# A function must move at least this share of the scenario's base cost to be
# listed in its breakdown.
FUNCTION_MIN_SHARE = 0.001
TOP_FUNCTIONS = 8
SCENARIO_ORDER = ["load", "layout", "repaint", "keystroke", "keystroke-marks", "export"]

# `path/file.rs:crate::module::function [binary]`, or `???:symbol`.
FUNCTION_LINE = re.compile(r"^\s*([\d,]+)\s+(?:\([\s\d.]+%\)\s+)?(\S.*)$")
FILE_PREFIX = re.compile(r"^(?:\?\?\?|[^:\s]+\.[A-Za-z0-9]+):")
RUST_HASH = re.compile(r"::h[0-9a-f]{16}")
BINARY_SUFFIX = re.compile(r"\s+\[[^\]]*\]$")
# Callgrind tells recursion levels apart as `name'2`; they are one function.
RECURSION_SUFFIX = re.compile(r"'\d+$")


def total_instructions(path: Path) -> int | None:
    for line in path.read_text(errors="replace").splitlines():
        if line.startswith(("summary:", "totals:")):
            return int(line.split()[1])
    return None


def function_costs(path: Path) -> dict[str, int]:
    """Exclusive instructions per function, keyed by a checkout-independent name."""
    output = subprocess.run(
        ["callgrind_annotate", "--threshold=100", "--auto=no", str(path)],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    costs: dict[str, int] = {}
    in_table = False
    for line in output.splitlines():
        if "file:function" in line:
            in_table = True
            continue
        if not in_table:
            continue
        match = FUNCTION_LINE.match(line)
        if not match:
            continue
        name = BINARY_SUFFIX.sub("", match.group(2).strip())
        name = FILE_PREFIX.sub("", name)
        name = RUST_HASH.sub("", name)
        name = RECURSION_SUFFIX.sub("", name)
        costs[name] = costs.get(name, 0) + int(match.group(1).replace(",", ""))
    return costs


def runs(directory: Path) -> dict[tuple[str, str], Path]:
    found = {}
    for path in directory.glob("*.out"):
        scenario, _, fixture = path.stem.partition("@")
        found[(scenario, fixture)] = path
    return found


def order(key: tuple[str, str]) -> tuple[str, int]:
    scenario, fixture = key
    rank = SCENARIO_ORDER.index(scenario) if scenario in SCENARIO_ORDER else len(SCENARIO_ORDER)
    return (fixture, rank)


def millions(value: int) -> str:
    return f"{value / 1e6:,.1f}M"


def percent(base: int, head: int) -> float:
    return (head - base) * 100.0 / base if base else 0.0


def marker(delta_pct: float) -> str:
    if delta_pct <= -BREAKDOWN_THRESHOLD_PCT:
        return "🟢"
    if delta_pct >= BREAKDOWN_THRESHOLD_PCT:
        return "🔴"
    return ""


def short(name: str, width: int = 90) -> str:
    name = name.replace("|", "\\|")
    return name if len(name) <= width else name[: width - 1] + "…"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("base", type=Path)
    parser.add_argument("head", type=Path)
    parser.add_argument("--base-label", default="base")
    parser.add_argument("--head-label", default="head")
    args = parser.parse_args()

    base_runs = runs(args.base) if args.base.is_dir() else {}
    head_runs = runs(args.head)
    lines = [HEADER, ""]
    lines.append(
        f"Instructions executed in each scenario's measured region "
        f"(`crates/jade-docx-perf`), {args.base_label} → {args.head_label}. "
        f"Counts are deterministic to well under {BREAKDOWN_THRESHOLD_PCT}%, "
        f"so smaller moves are noise only in the per-function breakdown."
    )
    lines.append("")
    if not base_runs:
        lines.append(f"> {args.base_label} could not be measured; showing {args.head_label} only.")
        lines.append("")

    lines.append("| fixture | scenario | base | head | Δ |")
    lines.append("|---|---|---:|---:|---:|")
    breakdowns = []
    for key in sorted(head_runs, key=order):
        scenario, fixture = key
        head_total = total_instructions(head_runs[key])
        base_path = base_runs.get(key)
        base_total = total_instructions(base_path) if base_path else None
        if head_total is None:
            lines.append(f"| {fixture} | {scenario} | | failed | |")
            continue
        if base_total is None:
            lines.append(f"| {fixture} | {scenario} | – | {millions(head_total)} | |")
            continue
        delta = percent(base_total, head_total)
        lines.append(
            f"| {fixture} | {scenario} | {millions(base_total)} | {millions(head_total)} "
            f"| {marker(delta)} {delta:+.2f}% |"
        )
        if abs(delta) >= BREAKDOWN_THRESHOLD_PCT:
            breakdowns.append((key, base_path, head_runs[key], base_total))

    for (scenario, fixture), base_path, head_path, base_total in breakdowns:
        base_costs = function_costs(base_path)
        head_costs = function_costs(head_path)
        moved = []
        for name in set(base_costs) | set(head_costs):
            before = base_costs.get(name, 0)
            after = head_costs.get(name, 0)
            if abs(after - before) >= base_total * FUNCTION_MIN_SHARE:
                moved.append((after - before, before, after, name))
        moved.sort(key=lambda row: -abs(row[0]))
        lines.append("")
        lines.append(f"<details><summary>{fixture} / {scenario}: functions that moved</summary>")
        lines.append("")
        lines.append("| Δ instructions | base | head | function |")
        lines.append("|---:|---:|---:|---|")
        for change, before, after, name in moved[:TOP_FUNCTIONS]:
            lines.append(
                f"| {change / 1e6:+,.2f}M | {millions(before)} | {millions(after)} | `{short(name)}` |"
            )
        if not moved:
            lines.append("| | | | (spread thinly; no single function moved ≥0.1%) |")
        lines.append("")
        lines.append("</details>")

    lines.append("")
    lines.append(
        "Callgrind outputs for both sides are in the `perf-callgrind` artifact; "
        "open them with `callgrind_annotate` or KCachegrind for the full profile."
    )
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
