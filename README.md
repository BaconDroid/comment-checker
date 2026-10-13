# comment-checker

> 100% vibe coded. zero comments in my code, zero comments in yours.
>
> **this entire repo - code, docs, readme, everything - was written by LLMs.**
> shoutout to claude opus 4.5.

A PostToolUse hook for Claude Code / OpenCode that yells at you when you write unnecessary comments.

Built with Rust + tree-sitter. Fast. Opinionated. No mercy.

## why

comments are code smell. if your code needs comments to be understood, your code sucks.

this hook watches every `Write`, `Edit`, `MultiEdit` and screams when it detects comments.

exceptions exist. BDD comments (`# given`, `# when`, `# then`), linter directives (`# noqa`, `// @ts-ignore`), shebangs - these are fine. everything else? delete it. yes, even docstrings.

## install

### npm / bun

```bash
npm install -g @code-yeongyu/comment-checker
bun add -g @code-yeongyu/comment-checker
```

### homebrew (macos/linux)

```bash
brew tap code-yeongyu/tap
brew install comment-checker
```

### manual

grab binary from [releases](https://github.com/code-yeongyu/go-claude-code-comment-checker/releases).

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
pass. The wrapper resolves the binary, passes `0`/`2` through, and otherwise drains stdin (so a large
payload never raises SIGPIPE) and exits `1` with `did not run`. `tests/readme_hook.rs` runs this exact
command text, so weakening it (a bare command, `|| exit 0`, …) fails the build.

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
# given - BDD comments are fine
def test_something():
    # when
    result = do_thing()
    # then
    assert result == expected
```

```python
# noqa: E501 - linter directives are fine
```

```python
#!/usr/bin/env python - shebangs are fine
```

## 30+ languages

python, go, typescript, javascript, rust, c, c++, java, ruby, php, swift, kotlin, scala, elixir, and more.

if tree-sitter supports it, we support it. the production CLI is Rust.

## how it works

1. hook receives JSON from Claude Code
2. extracts content from `Write`/`Edit`/`MultiEdit` tool input
3. detects language from file extension
4. parses AST with tree-sitter
5. finds comment nodes
6. filters out allowed patterns (BDD, directives, shebangs)
7. if anything remains → exit 2 with warning message

## exit codes

| code | meaning |
|------|---------|
| 0 | pass - no comments found or skipped |
| 1 | the hook wrapper could not run the check (binary missing) - nothing was checked |
| 2 | warning - problematic comments detected |

## custom prompt

you can replace the default warning message with your own using `--prompt`:

```bash
comment-checker --prompt "Custom warning! {{comments}}"
```

use `{{comments}}` placeholder to insert the detected comments XML. if omitted, only your custom message is shown.

### with oh-my-opencode

```json
{
  "comment_checker": {
    "custom_prompt": "DETECTED:\n{{comments}}\nFix it."
  }
}
```

## philosophy

> "Code is like humor. When you have to explain it, it's bad." - Cory House

write self-documenting code. use meaningful names. extract functions. stop explaining what the code does and make the code explain itself.

## license

MIT. do whatever.
