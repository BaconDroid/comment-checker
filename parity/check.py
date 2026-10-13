#!/usr/bin/env python3
"""Cross-repo parity drift check.

`comment-checker` and `test-checker` are two separate single-package repos that share a surface
which must stay identical after normalizing the project name: build/config files, the CI and release
workflows and scripts, the npm wrapper, the CLI entrypoints, the shared engine, the shared
conformance suite, and this harness itself. This script reads (or clones) the sibling repo,
normalizes both sides, and fails on any divergence. It also checks each repo against a shared
API-shape + naming-convention contract. The domain layer's *content*
(`rules`/`detector`/`filters`/`models`/`output`/`api`, `README`, `AGENTS`) is intentionally excluded
from the byte diff, but its *shape* is checked.

Usage:
  python3 parity/check.py                                   # clone the sibling from GitHub
  PARITY_SIBLING=/path/to/sibling python3 parity/check.py   # use a local checkout
  PARITY_REF=<ref> python3 parity/check.py                  # clone a specific ref
"""
from __future__ import annotations

import difflib
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import NoReturn

REPOS = {
    "comment-checker": "https://github.com/BaconDroid/comment-checker.git",
    "test-checker": "https://github.com/BaconDroid/test-checker.git",
}

# (comment-checker token, test-checker token, placeholder). Order matters.
TOKENS = [
    (
        "Multi-language comment detection hook for Claude Code/OpenCode. Native binary powered by tree-sitter.",
        "Multi-language test-weakening detection hook for Claude Code/OpenCode. Native binary powered by tree-sitter.",
        "DESC",
    ),
    (
        "Multi-language comment detection hook for Claude Code and OpenCode",
        "Multi-language test-weakening detection hook for Claude Code and OpenCode",
        "DESC",
    ),
    ("ai.sisyphuslabs.comment-checker", "com.bacondroid.test-checker", "IDENT"),
    ("code-yeongyu/go-claude-code-comment-checker", "BaconDroid/test-checker", "REPO"),
    ("code-yeongyu/homebrew-tap", "BaconDroid/homebrew-tap", "HOMEBREW"),
    ("code-yeongyu/tap", "BaconDroid/tap", "TAP"),
    ("YeonGyu Kim <yeongyu.gim@gmail.com>", "BaconDroid", "AUTHOR"),
    ("0.8.0", "0.1.0", "VER"),
    ("CommentChecker", "TestChecker", "CHECKER_CLASS"),
    ("comment_checker", "test_checker", "CHECKER_U"),
    ("comment-checker", "test-checker", "CHECKER"),
    ("@code-yeongyu", "@bacondroid", "SCOPE"),
]

# Files/dirs compared between the two repos. Directories are walked recursively.
SHARED = [
    ".cargo/config.toml",
    ".gitignore",
    "clippy.toml",
    "deny.toml",
    "rustfmt.toml",
    "rust-toolchain.toml",
    ".github",
    "npm",
    "parity",
    "src/cli.rs",
    "src/engine.rs",
    "src/main.rs",
    "tests/parity.rs",
]

# Prefixes compared byte-for-byte (no name normalization): the shared harness and conformance suite.
IDENTICAL_PREFIXES = ("parity/", "tests/parity.rs")

# --- API-shape + naming-convention contract ------------------------------------------------------
# Canonical public item names: a per-repo alias table maps domain synonyms to one canonical name, so
# the same shape can be asserted on both repos.
ALIASES = {
    "CommentDetector": "Detector",
    "TestDetector": "Detector",
    "CommentInfo": "Item",
    "Finding": "Item",
    "CommentType": "Kind",
    "Confidence": "Kind",
    "build_comments_xml": "build_xml",
    "build_findings_xml": "build_xml",
    "detect_comments_json": "detect_json",
    "detect_findings_json": "detect_json",
}

# Module -> required canonical public items (both repos must expose exactly these, plus extras).
SHAPE = {
    "api": {
        "HookCheckResult",
        "pass",
        "block",
        "check_hook_input",
        "check_hook_input_with_options",
        "check_hook_input_from_reader",
        "check_hook_input_json",
        "detect_json",
    },
    "detector": {"Detector", "new", "detect", "detect_edit"},
    "filters": {
        "BddFilter",
        "DirectiveFilter",
        "ShebangFilter",
        "SuppressionFilter",
        "apply_filters",
        "should_skip",
        "is_agent_memo_phrase",
        "is_line_suppressed",
        "is_file_suppressed",
    },
    "language": {"language_for_path"},
    "models": {"Item", "Kind", "new", "normalized_text", "Edit"},
    "output": {"build_xml", "format_hook_message"},
    "lib": {"api", "cli", "detector", "engine", "filters", "language", "models", "output"},
}

# Documented per-repo extras (canonical names) beyond SHAPE.
EXTRAS = {
    "comment-checker": {
        "filters": {"AgentMemoFilter", "is_agent_memo"},
    },
    "test-checker": {
        "detector": set(),
        "filters": set(),
        "language": {"is_test_path", "is_harness_path"},
        "models": {"as_str"},
        "lib": {"rules"},
    },
}

# Required module stems (src/*.rs) and per-repo extras.
REQUIRED_MODULES = [
    "api",
    "cli",
    "detector",
    "engine",
    "filters",
    "language",
    "lib",
    "main",
    "models",
    "output",
]
MODULE_EXTRAS = {"comment-checker": set(), "test-checker": {"rules"}}

# Required test-file stems (tests/*.rs) and per-repo extras.
REQUIRED_TESTS = [
    "api",
    "cli",
    "detector",
    "filters",
    "integration",
    "models",
    "output",
    "parity",
    "readme_hook",
]
TEST_EXTRAS = {"comment-checker": set(), "test-checker": set()}

CAMEL_CASE = re.compile(r"^[A-Z][A-Za-z0-9]*$")
SNAKE_CASE = re.compile(r"^[a-z][a-z0-9_]*$")
PUBLIC_ITEM = re.compile(r"^\s*pub (?:const )?(fn|struct|enum|trait|type|mod)\s+([A-Za-z_][A-Za-z0-9_]*)", re.MULTILINE)


def normalize(text: str) -> str:
    for comment_token, test_token, placeholder in TOKENS:
        text = text.replace(comment_token, placeholder)
        text = text.replace(test_token, placeholder)
    return text


def detect_name(root: Path) -> str | None:
    manifest = (root / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^name\s*=\s*"([^"]+)"', manifest, re.MULTILINE)
    return match.group(1) if match else None


def shared_files(root: Path) -> dict[str, Path]:
    files: dict[str, Path] = {}
    for entry in SHARED:
        path = root / entry
        if path.is_file():
            files[normalize(entry)] = path
        elif path.is_dir():
            for candidate in sorted(path.rglob("*")):
                if candidate.is_file():
                    files[normalize(str(candidate.relative_to(root)))] = candidate
    return files


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def stems(directory: Path, suffix: str) -> set[str]:
    if not directory.is_dir():
        return set()
    return {p.stem for p in directory.glob(f"*{suffix}") if p.is_file()}


def canonical_public_items(path: Path) -> set[str]:
    items = set()
    for match in PUBLIC_ITEM.finditer(read(path)):
        items.add(ALIASES.get(match.group(2), match.group(2)))
    return items


def shape_violations(root: Path, repo: str) -> list[str]:
    problems: list[str] = []

    modules = stems(root / "src", ".rs")
    expected_modules = set(REQUIRED_MODULES) | MODULE_EXTRAS[repo]
    for missing in sorted(expected_modules - modules):
        problems.append(f"{repo}: src/{missing}.rs is missing")
    for unexpected in sorted(modules - expected_modules):
        problems.append(f"{repo}: src/{unexpected}.rs is not in the shape contract")

    for module, required in SHAPE.items():
        path = root / "src" / f"{module}.rs"
        if not path.is_file():
            continue
        actual = canonical_public_items(path)
        allowed = required | EXTRAS[repo].get(module, set())
        for missing in sorted(required - actual):
            problems.append(f"{repo}: src/{module}.rs must expose `{missing}`")
        for unexpected in sorted(actual - allowed):
            problems.append(f"{repo}: src/{module}.rs exposes unexpected `{unexpected}`")

    tests = stems(root / "tests", ".rs")
    expected_tests = set(REQUIRED_TESTS) | TEST_EXTRAS[repo]
    for missing in sorted(expected_tests - tests):
        problems.append(f"{repo}: tests/{missing}.rs is missing")
    for unexpected in sorted(tests - expected_tests):
        problems.append(f"{repo}: tests/{unexpected}.rs is not in the shape contract")

    benches = stems(root / "benches", ".rs")
    normalized_benches = {normalize(name) for name in benches}
    if normalized_benches != {"CHECKER_U"}:
        problems.append(f"{repo}: benches must be exactly one CHECKER_U target, found {sorted(benches)}")

    return problems


def naming_violations(root: Path) -> list[str]:
    problems: list[str] = []
    for directory, suffix in (("src", ".rs"), ("tests", ".rs"), ("benches", ".rs")):
        folder = root / directory
        if not folder.is_dir():
            continue
        for path in sorted(folder.glob(f"*{suffix}")):
            if not SNAKE_CASE.match(path.stem):
                problems.append(f"{root.name}: file {directory}/{path.name} is not snake_case")
    for path in sorted((root / "src").glob("*.rs")):
        for kind, name in PUBLIC_ITEM.findall(read(path)):
            if kind in {"struct", "enum", "trait", "type"} and not CAMEL_CASE.match(name):
                problems.append(f"{root.name}: {path.name} type `{name}` is not CamelCase")
            if kind in {"fn", "mod"} and not SNAKE_CASE.match(name):
                problems.append(f"{root.name}: {path.name} {kind} `{name}` is not snake_case")
    return problems


def diff_report(relative: str, local_text: str, sibling_text: str) -> str:
    diff = list(
        difflib.unified_diff(
            sibling_text.splitlines(),
            local_text.splitlines(),
            fromfile=f"sibling/{relative}",
            tofile=f"local/{relative}",
            lineterm="",
            n=1,
        )
    )
    return "\n".join(diff[:24])


def fail(message: str) -> NoReturn:
    print(f"parity: {message}", file=sys.stderr)
    sys.exit(1)


def main() -> int:
    local = Path.cwd()
    name = detect_name(local)
    if name is None or name not in REPOS:
        fail(f"unknown project name in Cargo.toml: {name!r}")
    repo: str = name

    sibling_name = next(key for key in REPOS if key != name)
    sibling_env = os.environ.get("PARITY_SIBLING")
    temp_dir: Path | None = None
    if sibling_env:
        sibling = Path(sibling_env)
    else:
        temp_dir = Path(tempfile.mkdtemp(prefix="parity-sibling-"))
        dest = temp_dir / "sibling"
        ref = os.environ.get("PARITY_REF")

        def clone(extra: list[str]) -> int:
            command = ["git", "clone", "--depth", "1", "--quiet", *extra, REPOS[sibling_name], str(dest)]
            return subprocess.run(command, check=False).returncode  # noqa: S603

        result = clone(["--branch", ref] if ref else [])
        if result != 0 and ref:
            # A coordinated change is pushed under the same branch name in both repos; if the
            # sibling does not carry that branch, fall back to its default branch.
            shutil.rmtree(dest, ignore_errors=True)
            result = clone([])
        if result != 0:
            fail(f"could not clone sibling repo {REPOS[sibling_name]}")
        sibling = dest

    if detect_name(sibling) != sibling_name:
        fail(f"{sibling} does not look like {sibling_name}")

    bootstrap = not (sibling / "parity").is_dir()
    local_files = shared_files(local)
    sibling_files = shared_files(sibling)

    drift: list[str] = []
    for relative in sorted(set(local_files) | set(sibling_files)):
        local_path = local_files.get(relative)
        sibling_path = sibling_files.get(relative)
        if local_path is None:
            drift.append(f"{relative}: present in sibling but missing locally")
            continue
        if sibling_path is None:
            drift.append(f"{relative}: present locally but missing in sibling")
            continue
        if relative.startswith(IDENTICAL_PREFIXES):
            if read(local_path) != read(sibling_path):
                drift.append(diff_report(relative, read(local_path), read(sibling_path)))
        elif normalize(read(local_path)) != normalize(read(sibling_path)):
            drift.append(diff_report(relative, normalize(read(local_path)), normalize(read(sibling_path))))

    contract = shape_violations(local, repo) + naming_violations(local)
    if not bootstrap:
        contract += shape_violations(sibling, sibling_name) + naming_violations(sibling)

    if temp_dir is not None:
        subprocess.run(["rm", "-rf", str(temp_dir)], check=False)  # noqa: S603

    if drift:
        print("parity drift detected between the two repos:")
        for report in drift:
            print()
            print(report)
        if bootstrap:
            print(
                "\nNOTE: the sibling has not adopted the parity harness yet, so this run is not "
                "failing. It will be enforced once both repos contain parity/."
            )
            return 0
        return 1

    if contract:
        print("parity: API-shape / naming contract violations:")
        for problem in contract:
            print(f"  - {problem}")
        return 1

    print("parity: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
