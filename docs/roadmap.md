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
| A1 | Severity per reason: `commented-out-code`, `agent-memo`, `past-facing`, `vacuous-todo` high; `restates-code`, `narrates-control-flow` medium (evidence depends on whole-file adjacency); `unnecessary-comment` low. | now |
| A2 | Honour `--min-confidence` (parsed today and ignored) and emit the tier on each `<comment>` element. A legacy tree can ratchet on proven smells only. | now |
| A3 | `comment-volume`: more than N comments added in one change, reported once at the first added comment, at low severity. | later |
| A4 | Duplicate and near-duplicate comments within a file, excluding spared classes and requiring a token floor. | later |

## Block B - Repo-level configuration

| # | Item | Status |
|:-:|:-----|:------|
| B1 | `.comment-checker.json` at the git root: `allow: [{glob, reasons, reason}]`, discovered through the first `.git` ancestor, entries dropped when malformed, a reason required. Exempts classes per directory instead of littering in-file markers. | now |
| B2 | Editing that config is itself a finding (`config-edited`). | next |
| B3 | `remove: [{glob, reasons, reason}]`, so `--strip` removes only approved reason classes and reports the rest as kept. | next |
| B4 | `scripts/doctor.sh` audit of legacy in-file markers. | later |

## Block C - Verdict quality

| # | Item | Status |
|:-:|:-----|:------|
| C1 | Opt-in `COMMENT_CHECKER_WRITE_BASELINE=1`: diff `HEAD` against the payload and report only comments on added lines. A large legacy file stops blocking forever on `Write`. | later |
| C2 | Reason-count metrics behind an env flag, plus a per-reason flag rate across the replay corpus, so the severity cutoffs are measured rather than asserted. | later |

## Block D - Evaluation

| # | Item | Status |
|:-:|:-----|:------|
| D1 | Precision replay over real comment history, modelled on the sibling's `eval/replay.py`: before/after binaries, per-reason counts, exit-2 payloads. | next |
| D2 | Secondary OSS corpus with a tracked manifest (URL, license, HEAD, commit and path per payload; payload bytes outside the checkout). | later |

## Coordinated with the sibling

| # | Item | Status |
|:-:|:-----|:------|
| **[cc+tc]** X1 | Marker hardening: a marker counts only inside a detected comment node, a marker added by the same edit is inert and reported, malformed markers report `allow-invalid`. `parity/vectors.json`'s `suppression-marker` case changes with it. | later |
| **[cc+tc]** X2 | Deterministic parse-budget outcome: identical bytes must always produce the same verdict. Needs an outcome decision inside `src/detector.rs` or a shared-surface change in `src/engine.rs`. | now |

## Not adopted

| Idea | Why |
|:-----|:----|
| Test-file admission semantics from the sibling | Comment analysis covers every code file; there is no analogue for harness routing or gate rules. |
| Line-scoped suppression markers | Would change the documented whole-fragment escape hatch beyond the intended `allow-added` change. |
| `path-predicates.json` contract | Exists to keep a duplicated path table in sync; this repo has a single extension lookup and no path table to publish. |
