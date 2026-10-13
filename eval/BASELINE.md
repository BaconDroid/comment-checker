# Baseline

Measured on this workstation (idle), release profile (`cargo build --release`), with
`eval/baseline.py` — 300 invocations of the trigger payload, process startup included.

| metric | value   | budget        |
| ------ | ------- | ------------- |
| p50    | 6.16 ms | —             |
| p95    | 8.08 ms | < 15 ms (met) |

The eight contract cases in `baseline.py` pass. Re-measure after any change to the detector, the
classifier, or `TSLP_LANGUAGES`. (Runs vary by ~1 ms; earlier runs measured p95 8.74–9.85 ms.)

`TSLP_LANGUAGES` grows the release binary: 31 languages = 53.8 MB, 34 (dart, haskell, r) = 59.4 MB
(+10.3%). Startup cost is unchanged (p95 within ~1 ms), so the budget is a parse/hook budget, not a
binary-size one.
