#!/usr/bin/env python3
"""Differential check: comments that `sf` flags but `comment-checker` does not.

`comment-checker` keeps flag-by-default and ports only part of sf's classifier, so it is expected to
flag **more** than sf. The number that matters is group (2): files sf flags that cc spares — a
potential miss.

Groups, per file:
  1. flagged by both;
  2. sf-flagged, cc-spared  (potential misses — listed);
  3. cc-flagged, sf-spared  (expected by design; summarised only).

Both tools are fed the same Write-style PostToolUse payload on stdin. Nothing is vendored: point
`--sf` at a checkout of the pinned sf commit and build both with `cargo build --release`.

Exits 0 when group (2) is at or below `--max-misses`, 1 otherwise, 2 on bad arguments.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

EXTENSIONS = {
    ".py", ".js", ".jsx", ".ts", ".tsx", ".go", ".rs", ".java", ".rb", ".php",
    ".c", ".h", ".cc", ".cpp", ".cs", ".kt", ".swift", ".scala", ".ex", ".exs",
    ".lua", ".sh", ".bash", ".sql", ".css", ".html", ".svelte", ".vue", ".dart",
    ".hs", ".r", ".zig", ".yaml", ".yml", ".toml",
}
SKIP_DIRS = {".git", "target", "node_modules", "dist", "build", ".venv", "__pycache__"}


def scan(roots: list[str]) -> list[Path]:
    files: list[Path] = []
    for root in roots:
        base = Path(root)
        if not base.exists():
            continue
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = [name for name in dirnames if name not in SKIP_DIRS]
            for name in filenames:
                path = Path(dirpath) / name
                if path.suffix.lower() in EXTENSIONS:
                    files.append(path)
    return sorted(set(files))


def flagged(binary: str, path: Path) -> bool:
    payload = json.dumps(
        {
            "tool_name": "Write",
            "tool_input": {
                "file_path": str(path),
                "content": path.read_text(encoding="utf-8", errors="replace"),
            },
        }
    ).encode()
    proc = subprocess.run([binary], input=payload, capture_output=True)
    return proc.returncode == 2


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default="target/release/comment-checker")
    parser.add_argument(
        "--sf",
        required=True,
        help="checkout of systemfsoftware/comment-checker at the pinned commit",
    )
    parser.add_argument(
        "--root",
        action="append",
        default=[],
        help="directory to scan (repeatable; default: src and tests)",
    )
    parser.add_argument("--max-misses", type=int, default=10)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    sf_binary = Path(args.sf) / "target" / "release" / "comment-checker"
    if not Path(args.cc).is_file():
        print(f"diff_vs_sf: missing cc binary {args.cc}", file=sys.stderr)
        return 2
    if not sf_binary.is_file():
        print(f"diff_vs_sf: missing sf binary {sf_binary}", file=sys.stderr)
        return 2

    roots = args.root or ["src", "tests"]
    files = scan(roots)
    both: list[str] = []
    misses: list[str] = []
    cc_only: list[str] = []
    for path in files:
        cc_flagged = flagged(args.cc, path)
        sf_flagged = flagged(str(sf_binary), path)
        name = str(path)
        if cc_flagged and sf_flagged:
            both.append(name)
        elif sf_flagged:
            misses.append(name)
        elif cc_flagged:
            cc_only.append(name)

    report = {
        "files": len(files),
        "flagged_by_both": len(both),
        "sf_flagged_cc_spared": len(misses),
        "cc_flagged_sf_spared": len(cc_only),
        "threshold": args.max_misses,
        "misses": misses,
    }
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print(f"files scanned: {len(files)}")
        print(f"(1) flagged by both: {len(both)}")
        print(f"(2) sf-flagged, cc-spared: {len(misses)}  (threshold {args.max_misses})")
        print(f"(3) cc-flagged, sf-spared: {len(cc_only)}")
        if misses:
            print("\npotential misses:")
            for name in misses:
                print(f"  {name}")
    return 1 if len(misses) > args.max_misses else 0


if __name__ == "__main__":
    sys.exit(main())
