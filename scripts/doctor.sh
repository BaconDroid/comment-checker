#!/usr/bin/env bash
# Standalone installation doctor for comment-checker.
#
# Verifies the binary on PATH, the local hook wiring, and both gate directions.
# This is intentionally a script, not a CLI subcommand, and does not import
# crate internals.
set -uo pipefail

say() {
    printf 'doctor: %s\n' "$*"
}

fail() {
    printf 'doctor: FAIL: %s\n' "$1" >&2
    shift
    for line in "$@"; do
        printf 'doctor: %s\n' "$line" >&2
    done
    exit 1
}

print_captured() {
    local label=$1
    local file=$2

    if [ -s "$file" ]; then
        printf 'doctor: %s:\n' "$label"
        sed 's/^/doctor: | /' "$file"
    else
        printf 'doctor: %s: (empty)\n' "$label"
    fi
}

BIN=$(command -v comment-checker 2>/dev/null || true)
if [ -z "$BIN" ]; then
    fail "comment-checker is not on PATH." \
        "Install it with one of:" \
        "  npm install -g @code-yeongyu/comment-checker" \
        "  brew tap code-yeongyu/tap && brew install comment-checker" \
        "Or add the directory containing the binary to PATH, then rerun this script."
fi

say "binary: $BIN"

version_output=$("$BIN" --version 2>&1)
version_status=$?
if [ "$version_status" -ne 0 ]; then
    fail "comment-checker --version failed with exit $version_status." \
        "The binary on PATH may be broken or may not be the expected executable." \
        "Output was:" \
        "$version_output" \
        "Reinstall or point PATH at a working comment-checker build."
fi
printf '%s\n' "$version_output"

settings=.claude/settings.json
if [ -e "$settings" ]; then
    if [ ! -f "$settings" ]; then
        fail "$settings exists but is not a regular file." \
            "Replace it with the hook settings file from the README setup."
    fi

    grep -q 'comment-checker' "$settings"
    settings_status=$?
    if [ "$settings_status" -eq 0 ]; then
        say "hook wiring: $settings references comment-checker"
    elif [ "$settings_status" -eq 1 ]; then
        fail "$settings does not reference comment-checker." \
            "The hook file is present but never invokes the checker, so writes are not checked." \
            "Wire the PostToolUse Write|Edit|MultiEdit command to comment-checker (see README setup)."
    else
        fail "could not read $settings to verify hook wiring." \
            "Check its permissions and rerun this script."
    fi
else
    say "hook wiring: $settings not found; skipping check (not a failure)."
fi

WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/comment-checker-doctor.XXXXXX") ||
    fail "could not create a temporary directory for checking binary output." \
        "Set TMPDIR to a writable directory and rerun this script."

cleanup() {
    rm -rf -- "$WORK_DIR"
}
trap cleanup EXIT

run_gate() {
    local label=$1
    local expected=$2
    local payload=$3
    shift 3
    local stdout_file="$WORK_DIR/$label.stdout"
    local stderr_file="$WORK_DIR/$label.stderr"
    local status

    printf '%s\n' "$payload" | "$BIN" >"$stdout_file" 2>"$stderr_file"
    status=$?

    if [ "$status" -ne "$expected" ]; then
        printf 'doctor: FAIL: %s gate expected exit %s, got %s.\n' \
            "$label" "$expected" "$status" >&2
        local line
        for line in "$@"; do
            printf 'doctor: %s\n' "$line" >&2
        done
        print_captured "binary stderr" "$stderr_file" >&2
        print_captured "binary stdout" "$stdout_file" >&2
        exit 1
    fi

    say "$label gate: PASS (exit $status)"
    print_captured "$label binary stderr" "$stderr_file"
    print_captured "$label binary stdout" "$stdout_file"
}

bad_payload='{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"counter += 1\n# increment the counter\n"}}'
good_payload='{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"print(1)\n"}}'

run_gate "known-bad" 2 "$bad_payload" \
    "Expected exit 2 for a restating comment, so the edit is blocked." \
    "The installed binary may be stale, or a wrapper may be rewriting its exit code." \
    "Reinstall or rebuild comment-checker and confirm '$BIN' is the intended binary."

run_gate "known-good" 0 "$good_payload" \
    "Expected exit 0 for a write with no comments." \
    "A wrapper or a stale/broken binary may be blocking a clean payload." \
    "Reinstall or rebuild comment-checker and confirm '$BIN' is the intended binary."

say "OK: binary resolves, version prints, hook wiring checked, restating comment blocks (2), clean write passes (0)."
exit 0
