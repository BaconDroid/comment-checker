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
