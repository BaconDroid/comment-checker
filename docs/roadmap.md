# Roadmap

Planned work, in blocks. Each block is independently shippable; entries inside a
block are ordered. Status is `now`, `next` or `later`. Finished work leaves this
file.

Cross-repo items are marked **[cc+tc]**: they change `src/engine.rs`,
`src/cli.rs`, `src/main.rs`, `parity/**` or `parity/vectors.json`, which are
byte-identical across both repos (`parity/check.py`), so they ship as one wave on
the same branch name in both repositories.

## Block A - Severity and gating

| # | Item | Status |
|:-:|:-----|:------|


## Block B - Repo-level configuration

| # | Item | Status |
|:-:|:-----|:------|
| B4 | `scripts/doctor.sh` audit of legacy in-file markers. | later |


## Block C - Verdict quality

| # | Item | Status |
|:-:|:-----|:------|

## Block D - Evaluation

| # | Item | Status |
|:-:|:-----|:------|
| D2 | Secondary OSS corpus with a tracked manifest (URL, license, HEAD, commit and path per payload; payload bytes outside the checkout). | later |

## Not adopted

| Idea | Why |
|:-----|:----|
| Test-file admission semantics from the sibling | Comment analysis covers every code file; there is no analogue for harness routing or gate rules. |
| Line-scoped suppression markers | Would change the documented whole-fragment escape hatch beyond the intended `allow-added` change. |
| `path-predicates.json` contract | Exists to keep a duplicated path table in sync; this repo has a single extension lookup and no path table to publish. |
