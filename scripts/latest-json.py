#!/usr/bin/env python3
# SOT: updater-feed, latest-json, release-signatures
#
# WHAT:  Builds the latest.json the in-app updater polls, from the .sig files the
#        bundle jobs left in the workspace.
# WHY:   tauri-action writes this file on GitHub Actions; CircleCI has no
#        equivalent, and an updater feed assembled by hand is how a release ships
#        pointing at a URL that does not exist.
# HOW:   Every updater artifact is bundled next to a detached signature, so the
#        .sig files are the index: one per platform, named after the artifact.
#        The URL is where `gh release upload` just put that artifact.
# WHERE: .circleci/config.yml (publish job), src-tauri/tauri.conf.json (updater)

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import urllib.parse
from datetime import datetime, timezone

# staged/<dir> -> the updater platform keys that directory's artifact serves.
# The macOS bundle is universal, so one artifact answers for both architectures.
PLATFORMS: dict[str, tuple[str, ...]] = {
    "macos-universal": ("darwin-aarch64", "darwin-x86_64"),
    "linux-x86_64": ("linux-x86_64",),
    "linux-aarch64": ("linux-aarch64",),
    "windows-x64": ("windows-x86_64",),
    "windows-arm64": ("windows-aarch64",),
}

# Only these are updater artifacts. A .dmg / .deb / .rpm is a first install, not
# an update, and listing one here would hand the updater a file it cannot apply.
UPDATER_SUFFIXES = (".app.tar.gz", ".AppImage", ".msi", ".exe")


def main() -> int:
    parser = argparse.ArgumentParser(description="Assemble the updater feed.")
    parser.add_argument("--tag", required=True, help="release tag, e.g. v0.5.0")
    parser.add_argument("--staged", required=True, type=pathlib.Path)
    parser.add_argument("--repo", required=True, help="owner/name")
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--notes", default="")
    args = parser.parse_args()

    platforms: dict[str, dict[str, str]] = {}
    for directory, keys in PLATFORMS.items():
        source = args.staged / directory
        if not source.is_dir():
            continue
        for signature in sorted(source.glob("*.sig")):
            artifact = signature.with_suffix("")  # drop .sig
            if not artifact.name.endswith(UPDATER_SUFFIXES):
                continue
            url = "https://github.com/{}/releases/download/{}/{}".format(
                args.repo, urllib.parse.quote(args.tag), urllib.parse.quote(artifact.name)
            )
            entry = {"signature": signature.read_text().strip(), "url": url}
            for key in keys:
                if key in platforms:
                    print(f"warning: {key} already set, keeping the first", file=sys.stderr)
                    continue
                platforms[key] = entry

    if not platforms:
        # Failing here beats publishing a feed that silently updates nobody.
        print(f"no signed updater artifacts under {args.staged}", file=sys.stderr)
        return 1

    feed = {
        "version": args.tag.lstrip("v"),
        "notes": args.notes or f"DB Free {args.tag}",
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }
    args.out.write_text(json.dumps(feed, indent=2) + "\n")
    print(f"latest.json: {len(platforms)} platform(s) -> {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
