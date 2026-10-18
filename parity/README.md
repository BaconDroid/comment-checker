# Cross-repo parity harness

`comment-checker` and `test-checker` are separate single-package repos that share a surface which
must stay identical after normalizing the project name. This directory makes that relationship
explicit and enforceable:

- `vectors.json` — the shared hook-contract corpus. Each repo reads its own section and runs the
  same cases (`tests/parity.rs`); the two sections describe the same contract with the domain's
  trigger inputs.
- `check.py` — clones (or reads) the sibling repo, normalizes names, and fails on any divergence in
  the shared surface: build/config files, `.github/**` (CI, release, scripts), `npm/**`, `src/cli.rs`,
  `src/engine.rs`, `src/main.rs`, this directory, and `tests/parity.rs`.

The domain layer (`rules`/`detector`/`filters`/`models`/`output`/`api`, `README`, `AGENTS`) is
intentionally excluded — that is where the two projects differ.

## Run it

```bash
python3 parity/check.py                                    # clone the sibling from GitHub
PARITY_SIBLING=../comment-checker python3 parity/check.py  # use a local checkout
PARITY_REF=<ref> python3 parity/check.py                   # clone a specific ref
```

CI runs it as the `parity` job in `.github/workflows/ci.yml`.

## Keeping the pair in sync

When a shared file changes, make the identical change in the sibling repo in the same wave. If a
change is genuinely domain-specific, it does not belong in a shared file — move it behind the domain
layer or add a `TOKENS` entry in `check.py` for a legitimate per-repo string.

## Merge protocol

`check.py` prints a short digest of each side's shared files and the sibling's commit, so a drift
report names the exact sibling revision. When the two PRs are ready:

1. Merge both PRs on the **same day**.
2. Keep the working branch until **both** are merged — each CI clones the sibling by `PARITY_REF`.
3. After both land, confirm `PARITY_REF` falls back to the default branch (`master`/`main`) and both
   CI parity jobs pass there.
