# Baseline

Measured with `eval/baseline.py` on a lightly loaded dev box (load average ~2; re-record on an idle
machine for the tightest gate).

## Contract

The exit-code cases in `baseline.py` pass: pass/skip exit `0`, findings exit `2`, and a
malformed/empty payload fails open to `0` with a visible `[<tool>] Skipping:` line.

## Relative perf gate

Each run is 210 invocations of the trigger payload (the first 10 are discarded); the harness takes the
**lowest p95 of 3 runs** and **fails** when p95 regresses by more than **25 %** against
`eval/baseline.json` (`profiles.<profile>.p95_ms`). It fails closed on a missing, non-positive, or
non-finite baseline/tolerance. The 15 ms budget is a documented **target** — it prints a warning, not
a failure.

Re-record on an idle machine:

```bash
python3 eval/baseline.py target/release/comment-checker --record
```

Recorded (load ~2): release p95 12.40 ms. Binary size: 31 languages ≈ 53.8 MB, 35 (dart/haskell/r/zig)
≈ 60.2 MB (+11.9 %).
