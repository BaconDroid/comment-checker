# Agent notes: go-claude-code-comment-checker

The Rust crate at the repository root (`src/`, `tests/`, `benches/`, `npm/`) is the current implementation.

## Commands (the same ones CI runs, `.github/workflows/ci.yml`)
- Format: `cargo fmt --all -- --check`
- Lint: `cargo clippy -p comment-checker --all-targets --all-features -- -D warnings`
- Full test: `cargo test -p comment-checker --all-targets --all-features`
- Focused test: `cargo test -p comment-checker --test <name>`, where `<name>` is one of the files in `tests/*.rs`
- npm wrapper: `node --test npm/comment-checker/*.test.js`

## Release platform touchpoints
Each supported platform (`darwin-arm64`, `darwin-x64`, `linux-arm64`, `linux-x64`, `win32-x64`) is named in six places — keep them in sync: `npm/comment-checker/index.js` `SUPPORTED_PLATFORMS`, `npm/comment-checker/postinstall.js` `PLATFORM_MAP`, `npm/comment-checker/package.json` `optionalDependencies`, `.github/scripts/prepare-npm-packages.sh`, `.github/scripts/verify-npm-packages.sh`, and the `.github/workflows/release.yml` build matrix. `packages.test.js` ties the first three together.

## Dependency convention
Direct dependencies are exactly `serde` (data model + serialization), `serde_json` (wire I/O only), `tree-sitter` (parsing and matching, via queries), and `tree-sitter-language-pack` (language, grammar, and identification only). Do not add dynamic `serde_json::Value` modeling, `TreeCursor` comment walks, or the pack's `get_parser`/`process`/runtime download; comments and docstrings are collected with `tree-sitter` queries. Keep direct dependencies to this set unless a change is justified and convention-following.

## Upstream
This fork is **not** merged or synced from upstream [`code-yeongyu/go-claude-code-comment-checker`](https://github.com/code-yeongyu/go-claude-code-comment-checker); the two have diverged (the archived `go/` tree and the `clap`/`itoa`/`criterion`/`tempfile` and N-API/wasm binding stacks are gone, and the detector is a query-based rewrite). Review upstream by hand and port only convention-following, non-superseded changes:

```bash
git remote add upstream https://github.com/code-yeongyu/go-claude-code-comment-checker.git
git fetch upstream && git log --oneline master..upstream/master
```

## Test authoring gate
Before adding or changing a test, answer all four; a missing answer means do not add it yet:
1. What observable behavior, invariant, or independent contract does it protect?
2. What credible regression makes it fail?
3. Why does existing coverage not already catch that failure? Each contract has one primary test owner at the strongest boundary; extend a table case or shared fixture instead of adding a near-duplicate.
4. Does it need a production seam (export, flag, wrapper, injection hook) no production caller needs? If yes, test at the real boundary instead.
Junk patterns (reject unless the retention bar below applies): assertion-free probes; self-comparisons and identity copiers; copied fixtures, inventories, manifests or export lists; exact source, import or string greps; private call-shape tests duplicated at a real boundary; duplicate invocations of one contract; replays of a shared helper through a wrapper; tests that exist to keep a test-only export or wrapper alive; production code whose only callers are tests; expected values produced by the code under test; mocks that implement the asserted behavior; fixture-supplied receipts or postconditions the code under test should produce; assertions against a store nothing writes; capability or flag restatements without a delivery proof; negative controls that pass for an unrelated reason; names that promise more than the input exercises.
Retention bar: keep a pattern match only when it independently guards a public API, protocol, config, migration, storage, security, platform, default, prompt-byte, generated or cross-language, package, release, or architecture contract, and say which one in the test. Static or slow is never a deletion reason. A retained test that fails on the base is a product bug to fix at its owner, never a test to delete.
A bug regression test must fail on the pre-fix code for the intended reason; an existing owner test that already fails may serve as that proof.
Method source: openclaw test-audit skill.
