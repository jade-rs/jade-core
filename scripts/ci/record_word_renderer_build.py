"""Bind the downloadable Windows coordinate probe to its actual CI source tree."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def record(root, executable, output, expected_head):
    root = root.resolve()
    executable, output = executable.resolve(), output.resolve()
    if not executable.is_relative_to(root) or not output.is_relative_to(root):
        raise ValueError("Renderer and provenance must stay inside the checkout")
    if output.exists():
        raise ValueError("Refusing to replace an existing renderer provenance record")
    if not expected_head:
        raise ValueError("Missing expected source commit")

    def git(*args):
        return subprocess.check_output(["git", *args], cwd=root, text=True).strip()

    # The PR checkout may be GitHub's synthetic merge commit. Record the actual
    # tree, and require the requested PR head to be part of that checkout.
    subprocess.run(["git", "merge-base", "--is-ancestor", expected_head, "HEAD"],
                   cwd=root, check=True)
    subprocess.run(["git", "diff", "--quiet", "HEAD", "--"], cwd=root, check=True)
    result = dict(
        schema=1, platform="windows", head_commit=git("rev-parse", "HEAD"),
        expected_head=git("rev-parse", expected_head + "^{commit}"),
        source_tree=git("rev-parse", "HEAD^{tree}"),
        executable=executable.relative_to(root).as_posix(),
        executable_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
        cargo_lock_sha256=hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest(),
        rustc=subprocess.check_output(["rustc", "-Vv"], cwd=root, text=True).strip(),
        cargo=subprocess.check_output(["cargo", "--version"], cwd=root, text=True).strip(),
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    record(Path(__file__).resolve().parents[2], args.executable, args.output,
           os.environ.get("WORD_RENDERER_EXPECTED_HEAD"))
