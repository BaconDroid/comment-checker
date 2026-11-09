# comment-checker

A PostToolUse hook for Claude Code / OpenCode that reports comments and docstrings an AI agent
should not have left behind.

## Why this exists

AI coding agents write code that another AI can read, run and debug. They still leave
narration behind: comments that restate the next line, trace their own work
("added X", "fix for Y"), comment out dead code, or park a vacuous TODO. That noise costs
tokens every time the file is read, drifts away from the code, and teaches the next agent
to imitate it.

comment-checker reports those comments and hands the agent a reason to fix each one. It is the
building block for a plugin that keeps AI-written code lean.

## What it does (and does not do)

`PostToolUse` runs **after** the tool has written the file. On a finding the binary exits `2` and
writes its message to stderr; Claude Code feeds that message back to the agent, which is expected
to revise the file. Without `--strip` the hook cannot prevent or undo the write, and it never edits
the file itself.

| | Status |
|---|---|
| Detect and report unnecessary comments | works today (PostToolUse, exit `2`) |
| Prevent or undo the write | not possible from `PostToolUse`; exit `2` is feedback after the write |
| Remove: strip flagged comments from a file | works today (`--strip`) |
| Plug in: one plugin combining this and test-checker | works today (`BaconDroid/opencode-comments-plugin` ships both engines) |

## Philosophy: narration vs intent

- **Flagged**: comments that restate or narrate the code, track the agent's own work,
  commented-out code, vacuous TODOs, agent memos.
- **Spared**: non-obvious intent (why, constraints, workarounds), license headers,
  generated-file markers, public API docs, attribution, linter directives, BDD markers.
- **Lean, not zero**: an AI can infer *what* code does, not *why* it is that way. A comment
  stays only for a *why* that still applies - a live constraint, a workaround with its
  trigger condition, an issue link that still binds the code - in one line. How the code got
  here belongs in the commit message; a behavior belongs in a test named after the bug; what
  the code already shows belongs in no comment at all. And if a comment is even slightly
  stale, it goes: a wrong comment costs more than none.

## install

GitHub Releases carry prebuilt binaries for the five platforms (`v0.8.0` at the time of writing);
they can lag the source tree, so build from source when you want the latest behavior. This fork
publishes no npm package or Homebrew formula. Build it from source:

```bash
git clone https://github.com/BaconDroid/comment-checker
cd comment-checker
cargo install --path . --locked
```

Rust 1.85+ and a C compiler are required (the tree-sitter grammars are C).

The upstream project publishes `@code-yeongyu/comment-checker`, a `code-yeongyu/tap` formula, and
binaries under `code-yeongyu/go-claude-code-comment-checker` releases. Those install upstream's
implementation, not this fork. The npm manifests under `npm/` still carry upstream's scope and
repository URL; they are unreleased packaging sources, not artifacts of this fork.

## setup

add to `~/.claude/settings.json` (or `.claude/settings.json` in your project):

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Write|Edit|MultiEdit",
        "hooks": [
          {
            "type": "command",
            "command": "if command -v comment-checker >/dev/null 2>&1; then exec comment-checker; fi; cat >/dev/null; echo '[check-comments] did not run - nothing checked this write.' >&2; exit 1"
          }
        ]
      }
    ]
  }
}
```

use the wrapper above, **not** a bare `comment-checker`. A bare command exits `127` without reading
stdin when the binary is not on `PATH`, so the edit goes unchecked and you are never told — a silent
pass. The wrapper resolves the binary, passes its exit code through, and only when the binary is
missing drains stdin (so a large payload never raises SIGPIPE) and exits `1` with `did not run`.
`tests/readme_hook.rs` runs this exact command text, so weakening it (a bare command,
`|| exit 0`, …) fails the build.

the wrapper assumes a POSIX shell: `command -v`, `cat`, and `>&2` must exist. On Windows without one,
wire the binary directly (`comment-checker.exe`) or run it through a shell that provides them; this
command text does not claim native Windows support.

done. now claude will think twice before leaving `// TODO: fix later` in your code.

## what it catches

```go
// bad: unnecessary comment
x := 1 + 1  // adds one to one

// bad: todo that will never be done
// TODO: refactor this later

// bad: commented out code
// fmt.Println("debug")
```

## what it allows

```python
# given
def test_something():
    # when
    result = do_thing()
    # then
    assert result == expected
```

BDD sparing is exact-keyword-only: after stripping one comment marker the text must equal one of
`given`, `when`, `then`, `arrange`, `act`, `assert`, `when & then`, `when&then` (ignoring case).
An annotation such as `# given - BDD comments are fine` would be flagged.

```python
# noqa: E501 - linter directives are fine
```

```python
#!/usr/bin/env python - shebangs are fine
```

it also classifies: a comment that carries a reason the code itself cannot show is spared too -

- a **license header** (`SPDX-License-Identifier`, `Copyright`, `Licensed under`, "permission is
  hereby granted", "all rights reserved", "redistribution and use", `GNU General Public License`,
  `Apache License`, `MIT License`, `BSD License`, `Mozilla Public License`)
- a **generated-file notice** (`Code generated by ...`, `DO NOT EDIT`, `auto-generated`,
  `autogenerated`, `generated by`)
- a **public-API docstring** containing contract markup: `@param`, `@returns`, `@return`, `@throws`,
  `@raises`, `@exception`, `@example`, `@see`, `@author`, `@deprecated`, `@since`, `@type`,
  `@typedef`, `@property`, `@brief`, `@details`, `Args:`, `Returns:`, `Raises:`, `Yields:`,
  `Attributes:`, `:param`, `:return:`, `:rtype:`, `:raises:`, `:type`, `# examples`, `# panics`,
  `# errors`, `# safety` - and only inside a docstring; a line comment leading with `@param` is
  still flagged
- an **attribution** (`@author`, `@copyright`, "adapted from", "based on", "ported from", "credit",
  `@see`, `@link`); `ref:`/`source:` count only when they start the comment, so prose that mentions
  them mid-sentence is not read as provenance
- a **non-obvious intent** (`why`, `because`, `workaround`, `important:`/`warning:`/`caution:`/
  `deprecated:`, a linked issue (`fixes #`/`issue #`/`bug #`), a URL (`see http`, `https://`,
  `http://`), `to avoid`/`to prevent`/`must not`/`should not`/`must be`, `algorithm`, `regex`,
  `security`, `thread-safety`/`thread safety`, `1-based`/`0-based`, `@link`)

the markers are matched as text, not parsed: an intent marker anywhere in the comment spares it,
unless the comment's own body proves a smell (`commented-out-code`, `vacuous-todo`, `agent-memo`) or
a whole-file `Write` cites adjacent code (`restates-code`, `narrates-control-flow`). The prose
keywords are a fallback, not proof: a narration that says "because" reports the proven reason.

a directive is not an acquittal: a `# noqa`-style directive does not excuse a comment whose body is
commented-out code, a vacuous `TODO`, an agent memo, or past-facing wording - a proven smell
outranks a directive.

`comment-checker: allow` is the exception: the hook checks it against the analyzed content before
detection runs, so it suppresses all findings for it - the whole `Write` content, or the edited
fragment for `Edit`/`MultiEdit`, including comments that would otherwise be proven violations. It
is an escape hatch for the analyzed content, not a per-line one, and it is not read from the rest
of the file on an edit.

## reasons

every reported comment is a `<comment>` element with a `reason` attribute:

| reason | meaning |
|---|---|
| `commented-out-code` | the body reads as commented-out code |
| `vacuous-todo` | a comment starting with `TODO`/`FIXME`; the rule does not judge whether the task is actionable |
| `agent-memo` | the comment tracks what the agent changed or implemented |
| `past-facing` | the comment narrates history ("preserve the existing behavior", "previously called") instead of stating the current behavior and its rationale |
| `unnecessary-comment` | the default: no spare rule matched |
| `restates-code: <evidence>` | whole-file `Write` only: the comment repeats adjacent code |
| `narrates-control-flow` | whole-file `Write` only: the comment narrates a nearby loop |

Detection is deny-by-default: a comment is reported unless a spare rule matches. On a whole-file
`Write` the contextual check can refine a generic reason and can overrule the prose-keyword
fallback; it cannot overrule a structural justification, a directive, or the opt-out marker. An
`Edit`/`MultiEdit` fragment has no trustworthy neighbours, so it keeps the text-level reason.

see [`CONCEPTS.md`](CONCEPTS.md) for the shared vocabulary (verdict, reason, evidence, unreliable,
floor, and the difference between fail-open and a silent pass), and [`NOTICE`](NOTICE) for upstream
attribution.

## languages

the binary compiles 35 grammars (`.cargo/config.toml`, `TSLP_LANGUAGES`): bash, c, cpp, csharp, css,
cue, dart, dockerfile, elixir, elm, go, groovy, haskell, hcl, html, java, javascript, kotlin, lua,
ocaml, php, proto, python, r, ruby, rust, scala, sql, svelte, swift, toml, tsx, typescript, yaml,
zig.

an unknown extension is skipped with `Skipping: Non-code file` and exits `0`. The same skip applies
when the pack maps an extension to a language outside the 35 above - for example `.md`, `.json`,
`.ini`, or `Makefile` - because this binary has no grammar to parse it. A shipped code language
whose grammar fails to load (a corrupt install) still reports `did not run` and exits `1`.

docstrings are detected for Python (`"""`/`'''` at line start), and for `/** … */` blocks in
JavaScript, TypeScript, TSX, and Java. Other languages report their comments only. A Python
docstring is inspected through its opening line, so markup on a later line is not seen.

## how it works

1. hook receives JSON from Claude Code (the OpenCode plugin sends the same shape, plus `apply_patch`)
2. extracts content from `Write`/`Edit`/`MultiEdit` tool input
3. detects language from file extension
4. parses AST with tree-sitter (50 ms budget per parse)
5. finds comment nodes, plus comments inside injected regions such as `<script>`
6. classifies: spares BDD, directives, shebangs, and justified comments (license, generated,
   public-API doc, attribution, non-obvious intent)
7. if anything remains → exit `2` with a warning message, each comment tagged with a `reason`

## environment

| variable | values | default |
|---|---|---|
| `COMMENT_CHECKER_KEYWORD_ACQUIT` | `proven-only`, `never` | `proven-only` |

`proven-only` keeps the prose-keyword allow-list as a fallback: `why`, `because`, `workaround`, a
URL, `algorithm`, `security`, and the other intent markers spare a comment unless the comment's own
body proves a smell or a whole-file `Write` cites adjacent code.

`never` disables the prose-keyword acquit entirely: a genuine rationale comment is reported on its
text-level reason (`unnecessary-comment`). Structural justifications (license, generated file,
public-API doc markup, attribution), directives, BDD markers, shebangs, and
`comment-checker: allow` still spare a comment. Missing, empty, or any unrecognized value is treated
as `proven-only`, and the variable is read on every run.

## CLI

| option | behavior |
|---|---|
| `--prompt <text>` | replace the default message. `{{comments}}` is replaced by the XML; without it the custom text is the whole message |
| `--strip` | after reporting, remove every flagged comment from the target file; all-or-nothing (see [strip](#strip)) |
| `--version` | print `comment-checker <version>` to stdout and exit `0` |
| `--min-confidence <level>` | accepted by the shared argument parser and ignored (comments carry no confidence) |

unknown arguments are ignored. On a hook run the message goes to stderr and stdout stays empty;
`--version` is the exception.

## exit codes

| code | meaning |
|------|---------|
| 0 | pass, or a deliberate skip (a skip prints `[check-comments] Skipping: <reason>`) |
| 2 | at least one finding remains |
| 1 | did not run - an internal failure, or the wrapper could not run the binary; nothing was checked |

malformed, empty, unknown-tool, missing-path, unknown-extension, and oversize payloads exit `0`
with a visible `Skipping:` line, so a payload the hook cannot read is not a silent pass. A panic is
not caught: release builds use `panic = "abort"`, so a crash aborts with a non-zero status.

## custom prompt

you can replace the default warning message with your own using `--prompt`:

```bash
comment-checker --prompt "Custom warning! {{comments}}"
```

use `{{comments}}` placeholder to insert the detected comments XML. if omitted, only your custom message is shown.

## strip

`--strip` runs a removal pass in the same invocation as the report. Findings are reported exactly as
without the flag and the exit code is unchanged (`0` clean, `2` findings, `1` did not run); the
message gains a trailing `[comment-checker] strip: ...` summary.

- **Whole-file mode.** The payload names the target file (`tool_input.file_path`/`filePath`, resolved
  against the payload `cwd`, falling back to the process cwd). When that file is on disk the run
  analyzes its full content, so the report is the one a whole-file `Write` would produce - a partial
  `Edit`/`MultiEdit` fragment is not enough to decide what to rewrite. When the file is missing the
  run falls back to the payload content and reports it without rewriting anything.
- **Removable set.** Every comment the run flags is removed. Spared classes (BDD, directives,
  shebangs, license/generated/public-API/attribution/intent) are never flagged, so they are never
  removed; a `comment-checker: allow` marker suppresses the run and leaves the file untouched.
  Flagged docstrings are removable too.
- **Rewrite rules.** A comment that owns its line removes the whole line including its ending. A
  trailing comment removes the comment and the whitespace before it, keeping the code and the line
  ending. A block comment or docstring removes its span; a line left empty by that removal is
  dropped. Everything else stays byte-identical.
- **All-or-nothing.** The stripped content is built in memory and written once. If a flagged comment
  cannot be located unambiguously in the on-disk content, or the write fails, the file is left
  untouched and the summary says why.

```bash
comment-checker --strip < hook.json
```

A plugin can call the binary this way after a write; `BaconDroid/opencode-comments-plugin` combines
this removal pass with `test-checker`.

## limits

- the prose-keyword allow-list is conservative outside a whole-file `Write`. In the default
  `proven-only` mode a comment containing `why`, `because`, `workaround`, a URL, `algorithm`, or
  `security` is still spared on an `Edit`/`MultiEdit` fragment, where no adjacent code can prove a
  restatement; a proven text smell (`commented-out-code`, `vacuous-todo`, `agent-memo`,
  `past-facing`) wins over the keyword, and on a whole-file `Write` so does cited adjacent code
  (`restates-code`, `narrates-control-flow`). With `COMMENT_CHECKER_KEYWORD_ACQUIT=never` the
  keyword fallback is gone for both whole files and fragments, so only the structural
  justifications, directives, BDD markers, shebangs, and the opt-out marker spare a comment. A
  comment starting with `allow` passes the directive filter.
- detection is per comment node. A comment that only makes sense next to code the hook never sees
  still lands in the default `unnecessary-comment` bucket.
- a Python docstring is matched by its opening line only; `@param`/`Args:` markup further down does
  not spare it.
- contextual `restates-code`/`narrates-control-flow` reasons are emitted only for a whole-file
  `Write`. Fragments keep the text-level reason.
- on a parse-budget abort the hook skips with `Skipping: Parse budget exceeded` and exits `0`. That
  is fail-open, not a proof that the file is clean. A mapped grammar the binary does not ship is a
  non-code skip (exit `0`); a grammar-load failure for a shipped code language is not fail-open: it
  exits `1` with `did not run`, because nothing was checked.
- in JavaScript, TypeScript, TSX, and Java, `/** ... */` docstrings are only inspected when the
  file parses cleanly; any parse error in the tree skips them silently. Python docstrings are
  line-based and unaffected.
- every Python line starting with `"""`/`'''` is treated as its own comment, so the closing
  delimiter line of a multi-line docstring can produce a spurious finding.
- without `--strip` this is a detector: it does not rewrite, delete, or format comments. With
  `--strip` it removes only the comments it flags, following the rules above.

## philosophy

> "Code is like humor. When you have to explain it, it's bad." - Cory House

write self-documenting code. use meaningful names. extract functions. stop explaining what the code does and make the code explain itself.

## license

MIT. do whatever.
