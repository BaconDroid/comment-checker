#!/usr/bin/env python3
"""D0 baseline: assert the binary's exit-code contract and measure per-invocation wall time.

Run from the repository root. This is an off-contract eval helper (see eval/README.md), not part
of the parity-locked suite.
"""
from __future__ import annotations

import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# (name, payload, expected exit code)
CASES = [
    ("clean", '{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"print(1)\\n"}}', 0),
    (
        "trigger",
        '{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"# comment\\nprint(1)\\n"}}',
        2,
    ),
    # D14: unreadable payload fails open (0), aligned with sf; the 1 is wrapper-only.
    ("invalid-json", "{not json", 0),
    ("empty-stdin", "", 0),
    # A realistic unknown-tool payload carries no content, so it skips (0). Note: vector 5 of the
    # target contract ("unknown tool -> skip") is not yet enforced for a content-bearing payload.
    ("unknown-tool", '{"tool_name":"Read","tool_input":{"file_path":"a.py"}}', 0),
    ("unknown-ext", '{"tool_name":"Write","tool_input":{"file_path":"a.zzz","content":"# c\\n"}}', 0),
    ("bdd-allowed", '{"tool_name":"Write","tool_input":{"file_path":"t.py","content":"# given\\nx=1\\n"}}', 0),
    ("noqa-allowed", '{"tool_name":"Write","tool_input":{"file_path":"t.py","content":"# noqa: E501\\nx=1\\n"}}', 0),
]

BUDGET_P95_MS = 15.0


def run(binary: Path, payload: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [str(binary)], input=payload.encode(), capture_output=True, check=False
    )


def main() -> int:
    binary = (
        Path(sys.argv[1])
        if len(sys.argv) > 1
        else ROOT / "target" / "release" / "comment-checker"
    )
    if not binary.is_file():
        print(f"baseline: binary not found: {binary}", file=sys.stderr)
        return 2

    failures = 0
    for name, payload, expected in CASES:
        result = run(binary, payload)
        status = "ok" if result.returncode == expected else "FAIL"
        if result.returncode != expected:
            failures += 1
        print(f"  [{status}] {name}: exit {result.returncode} (expected {expected})")

    trigger = next(payload for name, payload, _ in CASES if name == "trigger")
    samples = []
    for _ in range(300):
        start = time.perf_counter()
        run(binary, trigger)
        samples.append((time.perf_counter() - start) * 1000.0)
    samples.sort()
    p50 = statistics.median(samples)
    p95 = samples[int(len(samples) * 0.95)]
    if "debug" in binary.parts:
        note = "debug profile: the budget applies to release"
    else:
        verdict = "ok" if p95 < BUDGET_P95_MS else "OVER BUDGET"
        note = f"{verdict}, budget p95 < {BUDGET_P95_MS} ms"
    print(f"\nper-invocation (startup included): p50 {p50:.2f} ms, p95 {p95:.2f} ms [{note}]")

    if failures:
        print(f"\nbaseline: {failures} contract failure(s)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
