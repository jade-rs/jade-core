"""Resolve an optional release baseline without accepting mutable git refs."""
import os
from pathlib import Path
import re


def select_baseline(event_name, pull_request_base, dispatched_base):
    if event_name == "pull_request":
        baseline = pull_request_base
    elif event_name == "workflow_dispatch":
        if dispatched_base == "":
            return None
        baseline = dispatched_base
    else:
        raise ValueError(f"Unsupported performance event: {event_name}")
    if not re.fullmatch(r"[0-9a-f]{40}", baseline):
        raise ValueError("Performance baseline must be a full 40-character lowercase commit SHA")
    return baseline


if __name__ == "__main__":
    baseline = select_baseline(
        os.environ["GITHUB_EVENT_NAME"],
        os.environ.get("PR_BASE_SHA", ""),
        os.environ.get("DISPATCH_BASE_SHA", ""),
    )
    with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
        output.write(f"base_sha={baseline or ''}\nrequired={'true' if baseline else 'false'}\n")
