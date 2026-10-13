# Baseline

Measured on this workstation (idle), release profile (`cargo build --release`), with
`eval/baseline.py` — 300 invocations of the trigger payload, process startup included.

| metric | value   | budget        |
| ------ | ------- | ------------- |
| p50    | 6.37 ms | —             |
| p95    | 8.74 ms | < 15 ms (met) |

The eight contract cases in `baseline.py` pass: pass/skip exit `0`, findings exit `2`, and a
malformed or empty payload fails open to `0`. Re-measure after any change to the detector, the
classifier, or `TSLP_LANGUAGES`. (Runs vary by ~1 ms; an earlier run measured p95 9.85 ms.)
