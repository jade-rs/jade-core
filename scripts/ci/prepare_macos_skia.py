"""Use the verified 0.87 macOS archive instead of a network-dependent source fallback."""
import argparse
import hashlib
import os
from pathlib import Path
import tomllib
import urllib.request

VERSION = "0.87.0"
ASSET_URL = "https://api.github.com/repos/rust-skia/skia-binaries/releases/assets/266267944?download=1"
ASSET_SHA256 = "f15a955e61503f77d57d3cfee7362aabe93ea934d6580065a96de5519f2e7019"
LOCKS = (
    "crates/jade-skia-painter/Cargo.lock",
    "crates/jade-docx-native-render/Cargo.lock",
    "crates/jade-xlsx-native-render/Cargo.lock",
)


def verify_versions(root):
    for relative in LOCKS:
        lock = tomllib.loads((root / relative).read_text(encoding="utf-8"))
        versions = {}
        for package in lock["package"]:
            if package["name"] in ("skia-bindings", "skia-safe"):
                versions.setdefault(package["name"], set()).add(package["version"])
        if versions != {"skia-bindings": {VERSION}, "skia-safe": {VERSION}}:
            raise RuntimeError(f"Update the pinned Skia archive for {relative}: {versions}")


def prepare_archive(destination, expected_sha=ASSET_SHA256):
    destination = Path(destination)
    if destination.exists():
        data = destination.read_bytes()
    else:
        request = urllib.request.Request(
            ASSET_URL,
            headers={"Accept": "application/octet-stream", "User-Agent": "jade-native-ci"},
        )
        with urllib.request.urlopen(request, timeout=60) as response:
            data = response.read()
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected_sha:
        raise RuntimeError(f"Skia archive checksum mismatch: {actual}")
    if not destination.exists():
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    return destination.resolve().as_uri()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    verify_versions(Path(__file__).resolve().parents[2])
    uri = prepare_archive(args.output)
    # The archive includes both native libraries and their matching bindings.rs.
    # Do not force repository-mode downloads: this dependency is a registry crate.
    with Path(os.environ["GITHUB_ENV"]).open("a", encoding="utf-8") as env:
        env.write(f"SKIA_BINARIES_URL={uri}\n")
    print(f"Verified Skia {VERSION} macOS archive: {ASSET_SHA256}")


if __name__ == "__main__":
    main()
