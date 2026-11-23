#!/usr/bin/env python3
"""D1 precision replay: run real comment history through a before and an after binary.

Run from anywhere; it only reads git history and executes the binaries you name. It is an
off-contract eval helper (see eval/README.md): manual use only, not part of CI or the parity lock.

The corpus is the live git history of each `--repos` checkout, so it moves as the history grows:

1. **Replay** - for the 80 most recent non-merge commits that modify a shipped-language file, every
   modified file whose parent and commit blobs both exist yields one `Write` payload (the committed
   content) and one `Edit` payload (parent content -> committed content).
2. **Audit** - every tracked `.rs`/`.js`/`.py` file at `HEAD`, in sorted order, as a `Write`
   payload. `eval/*.py` is excluded.

Payloads over 2 MiB are skipped. The same payload list drives both binaries. Each run invokes
`[binary, "--min-confidence", "low"]`, counts exit-code-2 payloads, and counts the `reason`
attribute of every finding in the CLI findings on stderr.

Usage:
    python3 eval/replay.py --after target/release/comment-checker
    python3 eval/replay.py --before /tmp/before --after target/release/comment-checker --json
"""

from __future__ import annotations

import argparse
import collections
import html
import json
import re
import subprocess
import sys
from pathlib import Path

COMMITS = 80
MAX_PAYLOAD_BYTES = 2 * 1024 * 1024
MIN_CONFIDENCE = "low"

# The shipped-language set shared with eval/diff_vs_sf.py.
EXTENSIONS = {
    ".py", ".js", ".jsx", ".ts", ".tsx", ".go", ".rs", ".java", ".rb", ".php",
    ".c", ".h", ".cc", ".cpp", ".cs", ".kt", ".swift", ".scala", ".ex", ".exs",
    ".lua", ".sh", ".bash", ".sql", ".css", ".html", ".svelte", ".vue", ".dart",
    ".hs", ".r", ".zig", ".yaml", ".yml", ".toml",
}
AUDIT_EXTENSIONS = {".rs", ".js", ".py"}

# `reason` is the public output key; it is read from the opening tag of each finding so a comment
# whose text mentions `reason="..."` is never miscounted (text is XML-escaped, tags are not).
FINDING_REASON = re.compile(r'<comment line-number="\d+" reason="([^"]*)"')


class ReplayError(Exception):
    """A usage or input failure: the corpus or a binary could not be used."""


def git(repo: Path, *args: str) -> bytes:
    try:
        return subprocess.run(
            ["git", "-C", str(repo), *args], capture_output=True, check=True
        ).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        raise ReplayError(f"git {' '.join(args)} failed in {repo}: {error}") from error


def blob(repo: Path, revision: str, path: str) -> str | None:
    try:
        content = subprocess.run(
            ["git", "-C", str(repo), "cat-file", "blob", f"{revision}:{path}"],
            capture_output=True,
            check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    return content.decode("utf-8", errors="replace")


def suffix(path: str) -> str:
    return Path(path).suffix.lower()


def encode(tool_name: str, tool_input: dict[str, str]) -> bytes | None:
    payload = json.dumps({"tool_name": tool_name, "tool_input": tool_input}).encode()
    return payload if len(payload) <= MAX_PAYLOAD_BYTES else None


def replay_payloads(repo: Path) -> list[bytes]:
    commits = git(repo, "log", "--no-merges", "--format=%H").decode().split()
    payloads: list[bytes] = []
    replayed = 0
    for commit in commits:
        if replayed >= COMMITS:
            break
        try:
            changed = git(
                repo, "diff-tree", "--no-commit-id", "--name-only", "-r", "-z",
                "--diff-filter=M", f"{commit}^", commit,
            )
        except ReplayError:
            continue  # root commit: no parent to diff against
        paths = sorted(p for p in changed.decode().split("\0") if p and suffix(p) in EXTENSIONS)
        pairs = []
        for path in paths:
            old = blob(repo, f"{commit}^", path)
            new = blob(repo, commit, path)
            if old is not None and new is not None:
                pairs.append((path, old, new))
        if not pairs:
            continue
        replayed += 1
        for path, old, new in pairs:
            for payload in (
                encode("Write", {"file_path": path, "content": new}),
                encode("Edit", {"file_path": path, "old_string": old, "new_string": new}),
            ):
                if payload is not None:
                    payloads.append(payload)
    return payloads


def audit_payloads(repo: Path) -> list[bytes]:
    tracked = sorted(p for p in git(repo, "ls-files", "-z").decode().split("\0") if p)
    payloads: list[bytes] = []
    for path in tracked:
        if suffix(path) not in AUDIT_EXTENSIONS:
            continue
        if path.startswith("eval/") and path.endswith(".py"):
            continue
        content = blob(repo, "HEAD", path)
        if content is None:
            continue
        payload = encode("Write", {"file_path": path, "content": content})
        if payload is not None:
            payloads.append(payload)
    return payloads


def parse_reasons(stderr: str) -> list[str]:
    return [html.unescape(reason) for reason in FINDING_REASON.findall(stderr)]



METRICS_ENV = "COMMENT_CHECKER_METRICS"


def flag_rates(result: dict[str, object]) -> dict[str, float]:
    """Flagged findings per payload, per reason; the denominator is every payload."""
    payloads = result["payloads"]
    per_reason = result["per_reason"]
    assert isinstance(payloads, int) and isinstance(per_reason, dict)
    if payloads == 0:
        return {}
    return {reason: round(count / payloads, 4) for reason, count in sorted(per_reason.items())}

def run(binary: str, payloads: list[bytes]) -> dict[str, object]:
    reasons: collections.Counter[str] = collections.Counter()
    exit2 = 0
    for payload in payloads:
        try:
            process = subprocess.run(
                [binary, "--min-confidence", MIN_CONFIDENCE],
                input=payload,
                capture_output=True,
                check=False,
            )
        except OSError as error:
            raise ReplayError(f"cannot run {binary}: {error}") from error
        if process.returncode == 2:
            exit2 += 1
        elif process.returncode != 0:
            raise ReplayError(f"{binary} exited {process.returncode}, expected 0 or 2")
        reasons.update(parse_reasons(process.stderr.decode("utf-8", errors="replace")))
    result = {
        "payloads": len(payloads),
        "exit2": exit2,
        "findings": sum(reasons.values()),
        "per_reason": dict(sorted(reasons.items())),
    }
    result["flag_rate"] = flag_rates(result)
    return result


def delta(before: dict[str, object], after: dict[str, object]) -> dict[str, object]:
    before_reasons = before["per_reason"]
    after_reasons = after["per_reason"]
    assert isinstance(before_reasons, dict) and isinstance(after_reasons, dict)
    return {
        "exit2": after["exit2"] - before["exit2"],  # type: ignore[operator]
        "per_reason": {
            reason: after_reasons.get(reason, 0) - before_reasons.get(reason, 0)
            for reason in sorted(before_reasons.keys() | after_reasons.keys())
        },
    }


def build_report(
    replay: list[bytes], audit: list[bytes], before: str | None, after: str
) -> dict[str, object]:
    report: dict[str, object] = {
        "corpus": {"replay_payloads": len(replay), "audit_payloads": len(audit)}
    }
    results = {}
    if before is not None:
        results["before"] = {"replay": run(before, replay), "audit": run(before, audit)}
    results["after"] = {"replay": run(after, replay), "audit": run(after, audit)}
    report.update(results)
    if "before" in results:
        report["delta"] = {
            section: delta(results["before"][section], results["after"][section])
            for section in ("replay", "audit")
        }
    return report


def render(report: dict[str, object]) -> str:
    corpus = report["corpus"]
    assert isinstance(corpus, dict)
    lines = [
        f"corpus: {corpus['replay_payloads']} replay payloads, "
        f"{corpus['audit_payloads']} audit payloads"
    ]
    for label in ("before", "after"):
        if label not in report:
            continue
        lines.append(f"\n== {label} ==")
        for section, result in report[label].items():  # type: ignore[union-attr]
            lines.append(
                f"{section}: {result['payloads']} payloads, {result['exit2']} exit 2, "
                f"{result['findings']} findings"
            )
            lines.extend(f"  {reason}: {count}" for reason, count in result["per_reason"].items())
            if result.get("flag_rate"):
                lines.append("  per-reason flag rate (flagged/payloads):")
                lines.extend(
                    f"    {reason}: {rate}"
                    for reason, rate in result["flag_rate"].items()
                )
            if not result["per_reason"]:
                lines.append("  (no findings)")
    if "delta" in report:
        lines.append("\n== delta (after - before) ==")
        for section, change in report["delta"].items():  # type: ignore[union-attr]
            lines.append(f"{section}: {change['exit2']:+d} exit 2")
            lines.extend(f"  {reason}: {count:+d}" for reason, count in change["per_reason"].items())
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    parser.add_argument("--after", required=True, help="post-change binary")
    parser.add_argument("--before", help="pre-change binary; enables the delta")
    parser.add_argument("--repos", nargs="+", default=["."], help="git checkouts (default: .)")
    parser.add_argument("--json", action="store_true", help="print the report as JSON to stdout")
    args = parser.parse_args()

    try:
        replay: list[bytes] = []
        audit: list[bytes] = []
        for repo in map(Path, args.repos):
            replay.extend(replay_payloads(repo))
            audit.extend(audit_payloads(repo))
        if not replay and not audit:
            raise ReplayError("the corpus is empty: no replayable commits or tracked files")
        report = build_report(replay, audit, args.before, args.after)
    except ReplayError as error:
        print(f"replay: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2) if args.json else render(report))
    return 0


if __name__ == "__main__":
    sys.exit(main())
