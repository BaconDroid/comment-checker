# Concepts

The vocabulary shared by `comment-checker` and `test-checker`. The two hooks differ in domain (bad
comments vs bad tests) but agree on the meaning of "finding", "reason", "unreliable", "floor", and
on the difference between a deliberate spare and a hook that never ran.

## Lean (the product goal)

The tools exist to keep AI-written repositories small. An agent can derive *what* the code
does by reading and running it; it cannot derive *why*, and it starts every session without
memory. So the target is "lean", not "empty".

- **Narration** (comment): restates or describes the code, or tracks the agent's own work.
  Flagged.
- **Intent** (comment): a constraint, workaround, license, public API doc or non-obvious
  reason the code cannot show. Spared.
- **Scaffolding** (test): left over from the agent's own debugging or exploration (probes,
  duplicates, tautologies, hollow tests, source greps, mocks of the unit under test).
  Superfluous.
- **Guard** (test): pins behavior that must not regress (a fixed bug, a boundary, a
  contract). Kept, with its intent readable from its name.
- **Integrity**: a test must never be weakened, skipped, focused or deleted to get green.
  This holds even when the suite is lean.
- **Escape hatch**: the `comment-checker: allow` marker suppresses findings for the file, but it
  does not acquit a proven violation: a comment whose own body is commented-out code, a vacuous
  `TODO`, or an agent memo stays flagged.

## Verdict and kind

A **finding** is one reported problem: `comment-checker` reports a `<comment>` element,
`test-checker` a `<finding>` element. A **kind** (called `reason` on a comment and `rule` on a
finding) names *why* the element was reported. Naming the reason never changes *whether* something is
reported — it only tells the reader and the agent what to do about it.

## Evidence

A contextual verdict must be able to cite the thing it condemns. `comment-checker` cites the evidence
in the comment's `reason`: the tokens a comment shares with adjacent code, or a verb→operator pair
(`restates-code: increment +=`), plus the flow narration it names (`narrates-control-flow`). A verdict
that cannot cite such evidence is **not** emitted as a contextual one — the comment keeps its
text-level reason. Evidence is cited only from a trustworthy context (a whole-file `Write`); an
`Edit`/`MultiEdit` fragment keeps the text-level reason. This keeps the message checkable instead of a
bare accusation.

## Unreliable context

A `Write` gives the whole file, so a comment's neighbours are trustworthy. An `Edit`/`MultiEdit`
gives only a fragment, so the surrounding code a comment appears to restate may be missing or
misplaced. That context is **unreliable**: context-dependent verdicts must not be emitted on it. In
practice the contextual kind (`restates-code`, `narrates-control-flow`) is simply not emitted there;
the comment is still reported on its text-level reason. Purely textual rules (a `TODO`,
commented-out code, a skip marker) still apply, because they need no context to be true.

`comment-checker` reports every comment that is neither justified nor proven benign, and a kind
refines *why* it is reported — it does not gate whether it is. A repo may instead report only the
kinds it can prove; the choice is a product decision, recorded here so the two hooks do not drift.

## Floor

A **floor** is a per-kind minimum on the evaluation corpus, not an aggregate. Precision and recall
are computed per kind and each kind whose ground-truth bucket is large enough must clear the floor,
so a regression in one kind cannot hide inside a healthy overall score. A kind with no predictions
scores zero rather than a vacuous perfect.

## fail-open vs silent pass

These are opposites and must not be confused:

- **fail-open** — the *classifier* deliberately declines to act on something it cannot judge, and
  says so. This is correct behaviour: a conservative spare.
- **silent pass** — the *hook* exits success while having checked nothing: the binary was missing,
  the input never arrived, or a failure was swallowed. This is a defect.

`comment-checker` and `test-checker` fail open on a payload they cannot read (exit `0` with a
`Skipping:` line). The exit `1` "did not run" path belongs to the **wrapper**, never to the binary,
and the wrapper is documented and gated by `tests/readme_hook.rs`.

## Exit codes

| code | meaning |
| ---- | ------- |
| `0` | pass, or a deliberate skip. A skip **always** prints a visible `[<tool>] Skipping: <reason>` line on stderr — never a silent exit. |
| `2` | at least one finding remains — a comment that is neither justified nor proven benign. |
| `1` | **wrapper only**: the check did not run (binary missing, or an unexpected exit code). The binary itself never exits `1`. |

The malformed / empty / unknown-tool / missing-path / unknown-extension cases all exit `0` **with**
the visible `[<tool>] Skipping: <reason>` line, and `tests/parity.rs` asserts that line from
`parity/vectors.json`. Making a missing binary loud (exit `1`, "did not run") is the wrapper's job, so
a broken install can never look like a clean pass.

A crash is also non-zero: release builds use `panic = "abort"` and no `catch_unwind` (or panic hook)
is installed (`Cargo.toml`), so a panic aborts (SIGABRT, a non-zero status) instead of exiting `0`;
the wrapper maps any unexpected status to `1` ("did not run").
