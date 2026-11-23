# eval

Off-contract evaluation assets. `parity/` locks the cross-repo wire contract; `eval/` is
deliberately **outside** that lock (the parity harness only compares `SHARED` entries), so it may
grow per-repo fixtures, regression cases, and measured baselines without a coordinated wave.

`baseline.py` is the D0 regression guard: it drives the built binary over a fixed set of hook
payloads, asserts each one's exit code, and measures per-invocation wall time (process startup
included - that is what a PostToolUse hook pays). Use it before/after a detector or classifier
change to catch unintended behaviour or budget regressions.

## Recording policy

The gate is authoritative for the tagged release tree. A pull request does not re-record the
baseline, and a source change that leaves `built_from` stale is not a reason to edit
`eval/baseline.json`.

Re-recording happens in the release block (`docs/roadmap.md`) immediately before tagging, on the
merge commit; the tree measured is the tree that ships. `--record` writes that commit to
`built_from`.

If the gate refuses for provenance, do not re-record on the branch. Rebuild from the release tag's
merge commit, re-record once, and commit `eval/baseline.json` in the release PR.

```bash
cargo build --release
python3 eval/baseline.py                       # asserts + timing on target/release/comment-checker
python3 eval/baseline.py path/to/binary        # explicit binary
```

Exit codes are the contract: `0` clean or deliberate skip, `2` findings, `1` did not run (internal
failure, or the hook wrapper could not run the binary).

## Differential against sf

`diff_vs_sf.py` is the compatibility check against the reference classifier
(`systemfsoftware/comment-checker`). It does not build either tool; build both with
`cargo build --release` and point `--sf` at a checkout of the pinned commit. It feeds both binaries
the same Write-style payload per file and reports:

1. flagged by both;
2. sf-flagged, cc-spared (potential misses; listed);
3. cc-flagged, sf-spared (expected by design, since cc keeps flag-by-default).

It exits `1` when group (2) exceeds `--max-misses` (default 10). CI runs it weekly and on demand
(`.github/workflows/sf-differential.yml`); it is **not** part of the PR gate.

The imported corpus and the `sf kind → cc expectation` mapping live in `sf-corpus/`.

```bash
cargo build --release -p comment-checker
python3 eval/diff_vs_sf.py --sf /path/to/sf-checkout --root src --root tests
```

Pin bump: update the commit in `sf-corpus/README.md`, re-copy `eval/corpus.json`, and triage the new
group (2).

## Reason metrics

`COMMENT_CHECKER_METRICS=1` (exactly `1`) appends one `[check-comments] metrics: <reason> <count>` line per reason to stderr; withheld findings use `withheld-config:<reason>` and `withheld-confidence:<reason>` keys. Output, findings and exit codes are unchanged, and anything else leaves stderr byte-identical.

`eval/replay.py` reports per-reason flag rates (flagged findings divided by every payload) beside the raw counts. `eval/test_replay_rates.py` checks that arithmetic against three literal payloads, so a denominator change fails there instead of drifting in the report. Manual use only, like the rest of `eval/`.

## Precision replay

`replay.py` (D1) measures what a detector or classifier change does to real comment history. It is
manual-only: it is not run by CI and is not part of the PR gate. It builds nothing; build the
binary under test first and keep a copy of the pre-change binary for `--before`.

```bash
cargo build --release --locked
python3 eval/replay.py --after target/release/comment-checker
python3 eval/replay.py --before /tmp/before --after target/release/comment-checker --json
```

`--repos` takes one or more git checkouts (default `.`). `--json` prints the report to stdout; the
script writes no files, and result JSON is not committed.

The corpus is the live git history of the checkouts, so it grows with the history and totals move
between commits. For a fixed checkout state it is deterministic, and one payload list drives both
binaries:

1. **Replay**: the 80 most recent non-merge commits that modify a shipped-language file (the
   extension set of `diff_vs_sf.py`). For each modified file whose parent and commit blobs both
   exist, one `Write` payload carries the committed content and one `Edit` payload carries the
   parent to commit pair.
2. **Audit**: every tracked `.rs`/`.js`/`.py` file at `HEAD`, sorted, as a `Write` payload,
   excluding `eval/*.py`.

Deleted or missing blobs and payloads over 2 MiB are skipped. Each payload runs
`<binary> --min-confidence low` with JSON on stdin. The report counts exit-code-2 payloads and
tallies findings by `reason`, the public output key, parsed from the CLI findings on stderr. For
`--before` and `--after` it also gives the after minus before delta for exit-2 payloads and per
reason, with stable ordering and no timestamps.

It exits nonzero only for usage or input failures (an unusable repo or binary, an empty corpus, or
a binary exit other than 0 or 2), never for findings.

D1 does not add a secondary OSS corpus; that is D2.
