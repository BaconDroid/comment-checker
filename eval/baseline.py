#!/usr/bin/env python3
"""D0 baseline: assert the exit-code contract and gate the ANALYSIS/VERSION p95 ratio.

Run from the repository root. This is an off-contract eval helper (see eval/README.md), not part of
the parity-locked suite.

Absolute p95 drifts with machine load, so the gate uses the ratio of the analysis-case p95 to the
`--version` p95 measured in the *same* session. Five sessions run in order (analysis, then
`--version`); the gate uses the median of the five ratios. A single noisy gate run does not fail:
the last three run ratios are kept in `eval/baseline-history.json`, and only two over-threshold
runs out of those three fail the gate.

`baseline.json` records the ratio and `built_from` (the commit the binary was built from). Recording
and comparing both refuse to run when the binary is stale or build inputs are dirty. The 15 ms p95
target stays a warning.
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
TOLERANCE_PERCENT = 35.0
SESSIONS = 5
RUN_INVOCATIONS = 210
DISCARD_FIRST = 10
HISTORY_LEN = 3
BASELINE_FILE = Path(__file__).resolve().parent / "baseline.json"
HISTORY_FILE = Path(__file__).resolve().parent / "baseline-history.json"

# A commit that touches any of these can change the binary. A later commit that only touches eval
# data leaves the built artifact equivalent, which is what makes recording before the data commit
# safe.
BUILD_INPUTS = (
    "src",
    "benches",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "build.rs",
    ".cargo",
)


class ProvenanceError(RuntimeError):
    """The binary under test cannot be tied to the checked-out source."""


def git(*args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=ROOT, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or f"exit {result.returncode}"
        raise ProvenanceError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout.strip()


def resolve_commit(revision: str) -> str:
    try:
        return git("rev-parse", "--verify", f"{revision}^{{commit}}")
    except ProvenanceError as error:
        raise ProvenanceError(f"cannot resolve commit {revision!r}") from error


def resolve_expected_commit() -> str:
    """`BASELINE_BUILT_FROM` when set, otherwise the checked-out HEAD."""
    configured = os.environ.get("BASELINE_BUILT_FROM", "").strip()
    if configured:
        return resolve_commit(configured)
    try:
        return resolve_commit("HEAD")
    except ProvenanceError as error:
        raise ProvenanceError(
            "cannot resolve HEAD; set BASELINE_BUILT_FROM to the commit the binary was built from"
        ) from error


def build_inputs_clean() -> bool:
    result = subprocess.run(
        ["git", "status", "--porcelain", "--", *BUILD_INPUTS],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise ProvenanceError(f"git status failed: {result.stderr.strip()}")
    return not result.stdout.strip()


def same_build_inputs(left: str, right: str) -> bool:
    result = subprocess.run(
        ["git", "diff", "--quiet", left, right, "--", *BUILD_INPUTS],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode not in (0, 1):
        raise ProvenanceError(f"git diff failed: {result.stderr.strip()}")
    return result.returncode == 0


def latest_build_input_epoch(commit: str) -> int:
    result = subprocess.run(
        ["git", "log", "-1", "--format=%ct", commit, "--", *BUILD_INPUTS],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise ProvenanceError(f"git log failed: {result.stderr.strip()}")
    stamp = result.stdout.strip()
    return int(stamp) if stamp else 0


def require_provenance(binary: Path, baseline: dict | None = None) -> str:
    """Return the commit the binary is accepted as built from, or refuse to run."""
    expected = resolve_expected_commit()
    if not build_inputs_clean():
        raise ProvenanceError(
            "build inputs are dirty; commit or stash them before recording or comparing"
        )

    if baseline is None:
        commit = expected
        try:
            head = resolve_commit("HEAD")
        except ProvenanceError:
            head = expected
        if commit != head and not same_build_inputs(commit, head):
            raise ProvenanceError(
                f"BASELINE_BUILT_FROM {commit[:12]} does not match HEAD {head[:12]} and the "
                "build inputs differ"
            )
    else:
        recorded = baseline.get("built_from")
        if not isinstance(recorded, str) or not recorded.strip():
            raise ProvenanceError(f"{BASELINE_FILE.name} has no built_from commit")
        commit = resolve_commit(recorded)
        if commit != expected and not same_build_inputs(commit, expected):
            raise ProvenanceError(
                f"baseline was built from {commit[:12]}, but HEAD is {expected[:12]} and the "
                "build inputs differ; rebuild and re-record"
            )

    newest_source = latest_build_input_epoch(commit)
    try:
        binary_mtime = binary.stat().st_mtime
    except OSError as error:
        raise ProvenanceError(f"cannot stat binary {binary}: {error}") from error
    if newest_source and binary_mtime + 1 < newest_source:
        raise ProvenanceError(
            f"binary {binary} predates the latest build-input commit "
            f"({commit[:12]}); rebuild before recording or comparing"
        )
    return commit


def load_json(path: Path, what: str) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise RuntimeError(f"cannot load {what} {path}") from error
    if not isinstance(value, dict):
        raise RuntimeError(f"{what} {path} must contain a JSON object")
    return value


def load_baseline() -> dict:
    """The recorded median ratio, tolerance, and build provenance."""
    return load_json(BASELINE_FILE, "baseline")


def valid_positive(value: object) -> float | None:
    if isinstance(value, (int, float)) and math.isfinite(value) and value > 0:
        return float(value)
    return None


def load_history(profile: str) -> list[float]:
    if not HISTORY_FILE.exists():
        return []
    history = load_json(HISTORY_FILE, "baseline history")
    if history.get("profile") != profile:
        return []
    ratios = history.get("ratios")
    if not isinstance(ratios, list):
        raise RuntimeError(f"{HISTORY_FILE.name} has no ratios list")
    parsed = []
    for value in ratios:
        ratio = valid_positive(value)
        if ratio is None:
            raise RuntimeError(f"{HISTORY_FILE.name} contains an invalid ratio")
        parsed.append(ratio)
    return parsed[-HISTORY_LEN:]


def write_history(profile: str, ratios: list[float]) -> None:
    payload = {"profile": profile, "ratios": [round(value, 6) for value in ratios]}
    HISTORY_FILE.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


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


def measure_p95(binary: Path, payload: str, args: tuple[str, ...] = ()) -> float:
    """RUN_INVOCATIONS calls, discard the warm-up, return the p95 wall time in ms."""
    samples = []
    for _ in range(RUN_INVOCATIONS):
        start = time.perf_counter()
        run(binary, payload, args=args)
        samples.append((time.perf_counter() - start) * 1000.0)
    samples = sorted(samples[DISCARD_FIRST:])
    if not samples:
        raise RuntimeError("no timing samples")
    return samples[min(len(samples) - 1, int(len(samples) * 0.95))]


def measure_sessions(binary: Path, trigger: str) -> list[tuple[float, float, float]]:
    """Five sessions, each measuring analysis first and `--version` second."""
    sessions = []
    for _ in range(SESSIONS):
        analysis = measure_p95(binary, trigger)
        version = measure_p95(binary, "", ("--version",))
        if version <= 0:
            raise RuntimeError("--version p95 is not positive")
        sessions.append((analysis, version, analysis / version))
    return sessions


def report_sessions(sessions: list[tuple[float, float, float]]) -> tuple[float, float]:
    """Print the per-session numbers and the spread; return (median ratio, median analysis p95)."""
    print("\nsessions (analysis p95 then --version p95, in the same session):")
    for index, (analysis, version, ratio) in enumerate(sessions, start=1):
        print(
            f"  {index}: analysis p95 {analysis:.2f} ms, --version p95 {version:.2f} ms, "
            f"ratio {ratio:.3f}"
        )

    ratios = [ratio for _, _, ratio in sessions]
    versions = [version for _, version, _ in sessions]
    analyses = [analysis for analysis, _, _ in sessions]
    median_ratio = statistics.median(ratios)
    median_analysis = statistics.median(analyses)
    ratio_spread = max(ratios) - min(ratios)
    ratio_spread_percent = ratio_spread / median_ratio * 100.0
    version_spread = max(versions) - min(versions)
    version_spread_percent = version_spread / statistics.median(versions) * 100.0
    print(
        f"\nratios: {', '.join(f'{ratio:.3f}' for ratio in ratios)}; "
        f"median {median_ratio:.3f}; spread {ratio_spread:.3f} "
        f"({ratio_spread_percent:.1f}% of median)"
    )
    print(
        f"--version p95: {min(versions):.2f}-{max(versions):.2f} ms "
        f"(spread {version_spread:.2f} ms, {version_spread_percent:.1f}%)"
    )
    if median_analysis >= BUDGET_P95_MS:
        print(
            f"WARNING: analysis p95 {median_analysis:.2f} ms is over the "
            f"{BUDGET_P95_MS:.0f} ms target (warning only)"
        )
    else:
        print(
            f"analysis p95 {median_analysis:.2f} ms is within the "
            f"{BUDGET_P95_MS:.0f} ms target"
        )
    return median_ratio, median_analysis


def record(profile: str, commit: str, sessions: list[tuple[float, float, float]]) -> int:
    median_ratio, _ = report_sessions(sessions)
    baseline = {
        "tolerance_percent": TOLERANCE_PERCENT,
        "built_from": commit,
        "profiles": {profile: {"ratio": round(median_ratio, 6)}},
    }
    BASELINE_FILE.write_text(json.dumps(baseline, indent=2) + "\n", encoding="utf-8")
    write_history(profile, [])
    print(
        f"\nrecorded {profile} ratio {median_ratio:.3f} from {commit[:12]} in "
        f"{BASELINE_FILE.name}; cleared {HISTORY_FILE.name}"
    )
    return 0


def gate(
    profile: str, commit: str, sessions: list[tuple[float, float, float]], baseline: dict
) -> int:
    ratio, _ = report_sessions(sessions)
    recorded = valid_positive(
        baseline.get("profiles", {}).get(profile, {}).get("ratio")
        if isinstance(baseline.get("profiles"), dict)
        else None
    )
    if recorded is None:
        raise RuntimeError(f"{BASELINE_FILE.name} has no usable {profile} ratio")
    tolerance = valid_positive(baseline.get("tolerance_percent"))
    if tolerance is None:
        raise RuntimeError(f"{BASELINE_FILE.name} has no usable tolerance")

    threshold = recorded * (1.0 + tolerance / 100.0)
    runs = (load_history(profile) + [ratio])[-HISTORY_LEN:]
    write_history(profile, runs)
    over = sum(1 for value in runs if value > threshold)
    failed = over >= 2

    status = "FAIL" if failed else "PASS"
    print(
        f"\ngate: recorded {recorded:.3f}, threshold {threshold:.3f} "
        f"(+{tolerance:.0f}%), last {len(runs)} run(s) "
        f"{', '.join(f'{value:.3f}' for value in runs)}; "
        f"{over}/{len(runs)} over -> {status} (fails at 2 of {HISTORY_LEN})"
    )
    if over == 1 and not failed:
        print("note: one of the last three runs was over threshold; the gate tolerates one.")
    print(f"provenance: binary built from {commit[:12]}")
    return 1 if failed else 0


def main() -> int:
    binary = (
        Path(sys.argv[1])
        if len(sys.argv) > 1
        else ROOT / "target" / "release" / "comment-checker"
    )
    if not binary.is_file():
        print(f"baseline: binary not found: {binary}", file=sys.stderr)
        return 2

    profile = "debug" if "debug" in binary.parts else "release"
    recording = "--record" in sys.argv

    try:
        baseline = load_baseline() if not recording else None
        commit = require_provenance(binary, baseline)
    except (ProvenanceError, RuntimeError) as error:
        action = "record" if recording else "compare"
        print(f"\nbaseline: refusing to {action}: {error}", file=sys.stderr)
        return 2

    failures = 0
    for name, payload, expected in CASES:
        result = run(binary, payload)
        status = "ok" if result.returncode == expected else "FAIL"
        if result.returncode != expected:
            failures += 1
        print(f"  [{status}] {name}: exit {result.returncode} (expected {expected})")

    if profile == "debug":
        print("\ndebug profile: timing is not gated")
        return 1 if failures else 0

    trigger = next(payload for name, payload, _ in CASES if name.startswith("trigger"))
    sessions = measure_sessions(binary, trigger)

    if recording:
        recorded = record(profile, commit, sessions)
        return 1 if failures else recorded

    if baseline is None:
        print("\nbaseline: missing baseline data", file=sys.stderr)
        return 2
    try:
        status = gate(profile, commit, sessions, baseline)
    except RuntimeError as error:
        print(f"\nbaseline: {error}", file=sys.stderr)
        return 2

    if failures:
        print(f"\nbaseline: {failures} contract failure(s)", file=sys.stderr)
        return 1
    return status


if __name__ == "__main__":
    sys.exit(main())
