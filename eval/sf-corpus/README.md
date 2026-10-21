# sf eval corpus (imported)

Source: <https://github.com/systemfsoftware/comment-checker>, licensed **Apache-2.0**.
Pinned commit: **`44e1c87`** (the checkout the corpus was imported from).
`corpus.json` is copied unmodified from that commit's `eval/corpus.json` (60 cases). The attribution
obligation is recorded in the repository `NOTICE`.

## Kind mapping (sf → comment-checker)

| sf `kind`            | comment-checker expectation |
| -------------------- | --------------------------- |
| `Shebang`            | spared                      |
| `LinterDirective`    | spared                      |
| `BddStep`            | spared                      |
| `LicenseHeader`      | spared                      |
| `GeneratedFile`      | spared                      |
| `PublicApiDoc`       | spared                      |
| `Attribution`        | spared                      |
| `NonObviousIntent`   | spared                      |
| `CommentedOutCode`   | `commented-out-code`        |
| `VacuousTodo`        | `vacuous-todo`              |
| `AgentMemo`          | `agent-memo`                |
| `RestatesCode`       | `restates-code`             |
| `NarratesControlFlow`| `narrates-control-flow`     |

## Use

The **differential** (`../diff_vs_sf.py`) is the compatibility check: it runs both binaries over real
files and reports the files sf flags that comment-checker spares (potential misses). This corpus is
the pinned, attributable input. Merging it into the hard per-kind floor test is deferred until the
current mismatches are triaged, so the floor does not silently bless a miss.

## Pin bump

1. Check out a new sf commit and update the hash above.
2. Re-copy that checkout's `eval/corpus.json` over `corpus.json`.
3. Re-run `python3 eval/diff_vs_sf.py --sf <checkout>` and triage group (2).
