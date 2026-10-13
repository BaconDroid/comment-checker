# Baseline

Measured on this workstation (idle), release profile (`cargo build --release`), with
`eval/baseline.py` — 300 invocations of the trigger payload, process startup included.

| metric | value   | budget                        |
| ------ | ------- | ----------------------------- |
| p50    | 7.51 ms | —                             |
| p95    | 9.85 ms | < 15 ms (met)                 |

The eight contract cases in `baseline.py` pass: pass/skip exit `0`, findings exit `2`, and a
malformed or empty payload fails open to `0`. Re-measure after any change to the detector, the
classifier, or `TSLP_LANGUAGES`.

(The earlier debug-profile runs — p95 ~16 ms — are not the budget's profile and are not recorded.)
