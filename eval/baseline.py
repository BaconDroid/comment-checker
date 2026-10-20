#!/usr/bin/env python3
"""D0 baseline: assert the exit-code contract and gate the ANALYSIS/VERSION p95 ratio.

Run from the repository root. This is an off-contract eval helper (see eval/README.md), not part of
the parity-locked suite.

Absolute p95 drifts with machine load, so the gate is the ratio of the analysis-case p95 to the
`--version` p95 measured in the same session. `--record` refuses to write a baseline when the two
`--version` runs differ by more than 30% (a noisy machine). The 15 ms p95 target stays a warning.
"""
from __future__ import annotations

import json
import math
import os
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
    ("invalid-json", "{not json", 0),
    ("empty-stdin", "", 0),
    ("unknown-tool", '{"tool_name":"Read","tool_input":{"file_path":"a.py"}}', 0),
    ("unknown-ext", '{"tool_name":"Write","tool_input":{"file_path":"a.zzz","content":"# c\\n"}}', 0),
    ("bdd-allowed", '{"tool_name":"Write","tool_input":{"file_path":"t.py","content":"# given\\nx=1\\n"}}', 0),
    ("noqa-allowed", '{"tool_name":"Write","tool_input":{"file_path":"t.py","content":"# noqa: E501\\nx=1\\n"}}', 0),
]

BUDGET_P95_MS = 15.0
NOISE_LIMIT = 0.30
BASELINE_FILE = Path(__file__).resolve().parent / "baseline.json"


def load_baseline() -> dict:
    """The recorded analysis/version p95 ratio and regression tolerance."""
    try:
        return json.loads(BASELINE_FILE.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise RuntimeError(f"cannot load baseline {BASELINE_FILE}") from error


def run(
    binary: Path,
    payload: str,
    env: dict[str, str] | None = None,
    args: tuple[str, ...] = (),
) -> subprocess.CompletedProcess[bytes]:
    # `binary` is this harness's own argument (a locally built executable), never untrusted input,
    # so invoking it is the tool's purpose rather than a command-injection sink.
    return subprocess.run(  # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit
        [str(binary), *args], input=payload.encode(), capture_output=True, check=False, env=env
    )


RUN_INVOCATIONS = 210
DISCARD_FIRST = 10

Timed = list[tuple[str, dict[str, str] | None, tuple[str, ...]]]


def measure_once(binary: Path, cases: Timed) -> tuple[float, float]:
    """One run: RUN_INVOCATIONS iterations of the worst timed case, return (median, p95) ms."""
    samples = []
    for _ in range(RUN_INVOCATIONS):
        worst = 0.0
        for payload, env, args in cases:
            start = time.perf_counter()
            run(binary, payload, env, args)
            worst = max(worst, (time.perf_counter() - start) * 1000.0)
        samples.append(worst)
    samples = samples[DISCARD_FIRST:]
    samples.sort()
    return statistics.median(samples), samples[int(len(samples) * 0.95)]


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

    trigger = next(payload for name, payload, _ in CASES if name.startswith("trigger"))
    profile = "debug" if "debug" in binary.parts else "release"

    analysis_cases: Timed = [(trigger, None, ())]
    cwd_case = next((payload for name, payload, _ in CASES if name == "write-with-cwd"), None)
    if cwd_case is not None:
        analysis_cases.append(
            (cwd_case, {**os.environ, "TEST_CHECKER_WRITE_BASELINE": "1"}, ())
        )
    # Three runs; take the run with the lowest p95 to cut noise.
    p50, p95 = min(
        (measure_once(binary, analysis_cases) for _ in range(3)), key=lambda pair: pair[1]
    )

    version_cases: Timed = [("", None, ("--version",))]
    version_a = min(
        (measure_once(binary, version_cases) for _ in range(3)), key=lambda pair: pair[1]
    )[1]
    version_b = min(
        (measure_once(binary, version_cases) for _ in range(3)), key=lambda pair: pair[1]
    )[1]
    version_p95 = min(version_a, version_b)
    variance = (
        abs(version_a - version_b) / max(version_a, version_b)
        if max(version_a, version_b) > 0
        else 0.0
    )
    ratio = p95 / version_p95 if version_p95 > 0 else 0.0

    if "--record" in sys.argv:
        if variance > NOISE_LIMIT:
            print(
                f"\nbaseline: refusing to record: --version p95 varies {variance:.0%} between two "
                f"runs (> {NOISE_LIMIT:.0%}); rerun on a quieter machine.",
                file=sys.stderr,
            )
            return 2
        baseline = load_baseline()
        baseline.setdefault("profiles", {})[profile] = {"ratio": round(ratio, 4)}
        baseline["tolerance_percent"] = 15.0
        BASELINE_FILE.write_text(json.dumps(baseline, indent=2) + "\n", encoding="utf-8")
        print(
            f"\nrecorded {profile} ratio {ratio:.2f} "
            f"(analysis p95 {p95:.2f} / version p95 {version_p95:.2f}) in {BASELINE_FILE.name}"
        )
        return 1 if failures else 0

    if profile == "debug":
        print(
            f"\nanalysis: p50 {p50:.2f} ms, p95 {p95:.2f} ms; --version p95 "
            f"{version_p95:.2f} ms; ratio {ratio:.2f} (variance {variance:.0%}) "
            "[debug profile: not gated]"
        )
        return 1 if failures else 0

    try:
        baseline = load_baseline()
    except RuntimeError as error:
        print(f"\nbaseline: {error}", file=sys.stderr)
        return 1

    # Ratio gate: fail when the analysis/version ratio regresses > tolerance vs the recorded value.
    recorded = baseline.get("profiles", {}).get(profile, {}).get("ratio")
    recorded = (
        float(recorded)
        if isinstance(recorded, (int, float)) and math.isfinite(recorded) and recorded > 0
        else None
    )
    tolerance = baseline.get("tolerance_percent", 15.0)
    tolerance = (
        float(tolerance)
        if isinstance(tolerance, (int, float)) and math.isfinite(tolerance) and tolerance > 0
        else None
    )
    regression = ((ratio - recorded) / recorded * 100.0) if recorded else 0.0
    # Fail-closed: a missing/invalid baseline or tolerance must fail, never disable the gate.
    regressed = recorded is None or tolerance is None or regression > tolerance
    flags = []
    if recorded is None:
        flags.append("no usable baseline")
    if tolerance is None:
        flags.append("no usable tolerance")
    if p95 >= BUDGET_P95_MS:
        flags.append(f"p95 over target {BUDGET_P95_MS:.0f} ms")
    if recorded is not None and tolerance is not None and regression > tolerance:
        flags.append(f"ratio regressed {regression:+.0f}% > {tolerance:.0f}%")
    if variance > NOISE_LIMIT:
        flags.append(f"noisy machine: --version variance {variance:.0%}")
    status = "; ".join(flags) if flags else "ok"
    print(
        f"\nanalysis: p50 {p50:.2f} ms, p95 {p95:.2f} ms; --version p95 {version_p95:.2f} ms; "
        f"ratio {ratio:.2f} (variance {variance:.0%}) [{status}]"
    )

    if failures or regressed:
        print(
            f"\nbaseline: {failures} contract failure(s), regressed={regressed}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
