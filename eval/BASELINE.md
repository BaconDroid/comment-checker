# Baseline

Measured with `eval/baseline.py` on a loaded dev box (load average ~8; re-record on a quieter
machine for the tightest gate).

## Contract

The exit-code cases in `baseline.py` pass: pass/skip exit `0`, findings exit `2`, and a
malformed/empty payload fails open to `0` with a visible `[<tool>] Skipping:` line.

## Relative perf gate

Each measurement session runs 210 invocations of the analysis payload, then 210 invocations of
`--version`; the first 10 invocations of each batch are discarded. Five sessions run in that order,
giving five ratios `p95(analysis) / p95(--version)`. The gate uses the **median ratio**.

`eval/baseline.json` records the median ratio, a **35 %** tolerance, and the `built_from` commit.
The allowed ratio is `recorded * 1.35`. A single noisy run does not fail the gate: every comparison
appends its ratio to `eval/baseline-history.json`, only the last three runs are kept, and the gate
fails when **two of those three** are over the allowed ratio. Recording clears the history.

`--record` and comparison both refuse with exit `2` when the binary cannot be tied to the checked-out
source: build inputs must be clean, the binary must not predate the latest build-input commit, and
either `BASELINE_BUILT_FROM` or `git rev-parse HEAD` must resolve to the recorded commit (eval-only
commits, which cannot change the binary, are accepted). The 15 ms analysis p95 target is a
**warning**, never a failure.

```bash
cargo build --release --locked
python3 eval/baseline.py target/release/comment-checker --record
python3 eval/baseline.py target/release/comment-checker   # gate
```

`BASELINE_BUILT_FROM=<commit> python3 eval/baseline.py ...` overrides the expected commit.

## Recorded

Built from `54ae69ad263e4e3792b9b73d6ad2ee8ebd651281` (the eval-only change does not alter build
inputs). Record run: analysis p95 13.09 ms, **median ratio 2.600**.

| Session | Analysis p95 (ms) | `--version` p95 (ms) | Ratio |
| --- | ---: | ---: | ---: |
| 1 | 13.70 | 5.27 | 2.600 |
| 2 | 12.94 | 4.98 | 2.597 |
| 3 | 13.09 | 5.01 | 2.610 |
| 4 | 13.37 | 5.09 | 2.626 |
| 5 | 13.00 | 5.04 | 2.580 |

Ratio spread: 0.046 (1.8 % of median). `--version` p95 spread: 0.29 ms (5.7 %).

Two consecutive gate runs passed: ratios 2.663 and 2.523 against an allowed 3.509.

Binary size: 31 languages ≈ 53.8 MB, 35 (dart/haskell/r/zig) ≈ 60.2 MB (+11.9 %).
