# eval

Off-contract evaluation assets. `parity/` locks the cross-repo wire contract; `eval/` is
deliberately **outside** that lock (the parity harness only compares `SHARED` entries), so it may
grow per-repo fixtures, regression cases, and measured baselines without a coordinated wave.

`baseline.py` is the D0 regression guard: it drives the built binary over a fixed set of hook
payloads, asserts each one's exit code, and measures per-invocation wall time (process startup
included — that is what a PostToolUse hook pays). Use it before/after a detector or classifier
change to catch unintended behaviour or budget regressions.

```bash
cargo build --release
python3 eval/baseline.py                       # asserts + timing on target/release/comment-checker
python3 eval/baseline.py path/to/binary        # explicit binary
```

Exit codes are the contract: `0` pass/skip, `2` findings. The binary never exits `1`; `1` is
reserved for the hook **wrapper** ("did not run", see the README wiring section).

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
