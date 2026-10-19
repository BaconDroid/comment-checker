//! Gatekeeper for the documented hook wiring.
//!
//! The `"command"` value in the README `settings.json` block is the only supported wiring. This
//! suite runs that exact text, so a bare `comment-checker`, a `|| exit 0`, or any other weakening
//! that turns a missing binary into a silent pass fails the build.
#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::{Command, Output, Stdio};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The command is the first `"command": "..."` value in the README. The wrapper avoids inner double
/// quotes, so the value needs no unescaping.
fn readme_command() -> Result<String, Box<dyn std::error::Error>> {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))?;
    let start = readme
        .find("\"command\":")
        .ok_or("README does not document a hook command")?;
    let rest = &readme[start + "\"command\":".len()..];
    let open = rest.find('"').ok_or("command value does not open")?;
    let rest = &rest[open + 1..];
    let close = rest.find('"').ok_or("command value does not close")?;
    Ok(rest[..close].to_string())
}

/// A PATH that cannot resolve the hook binary but still resolves the wrapper's `cat`.
fn path_without_the_binary() -> Result<String, Box<dyn std::error::Error>> {
    let cat = std::env::var("PATH")?
        .split(':')
        .map(|dir| Path::new(dir).join("cat"))
        .find(|candidate| candidate.is_file())
        .ok_or("cat is not on PATH")?;
    let dir = std::env::temp_dir().join(format!("readme-hook-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let link = dir.join("cat");
    let _ = std::fs::remove_file(&link);
    symlink(&cat, &link)?;
    Ok(dir.to_string_lossy().to_string())
}

fn run(
    command: &str,
    search_path: &str,
    input: &[u8],
) -> Result<Output, Box<dyn std::error::Error>> {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .env("PATH", search_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.as_mut().ok_or("stdin handle unavailable")?;
    // Fails if the wiring does not drain stdin: an unread pipe blocks or raises SIGPIPE.
    stdin.write_all(input)?;
    Ok(child.wait_with_output()?)
}

#[test]
fn documented_wiring_drains_stdin_and_exits_one_when_the_binary_is_missing() -> TestResult {
    // given: a PATH that cannot resolve the binary
    let command = readme_command()?;
    let payload = vec![b'x'; 1_500_000];

    // when
    let output = run(&command, &path_without_the_binary()?, &payload)?;

    // then
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("did not run"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn documented_wiring_passes_the_binary_exit_codes_through() -> TestResult {
    // given: the built binary on PATH
    let command = readme_command()?;
    let bin = env!("CARGO_BIN_EXE_comment-checker");
    let dir = Path::new(bin)
        .parent()
        .ok_or("binary has no parent")?
        .to_string_lossy()
        .to_string();

    // when: a clean payload, then a triggering payload
    let clean = run(
        &command,
        &dir,
        br#"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"print(1)"}}"#,
    )?;
    let trigger = run(
        &command,
        &dir,
        b"{\"tool_name\":\"Write\",\"tool_input\":{\"file_path\":\"a.py\",\"content\":\"# c\\nprint(1)\"}}",
    )?;

    // then
    assert_eq!(
        clean.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&clean.stderr)
    );
    assert_eq!(
        trigger.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&trigger.stderr)
    );
    Ok(())
}

/// The gatekeeper has teeth: each weakening the README warns against must fail the missing-binary
/// contract (exit 1 **and** "did not run"), so a future edit cannot silently neuter it.
#[test]
fn weakening_patterns_do_not_satisfy_the_missing_binary_contract() -> TestResult {
    let name = env!("CARGO_PKG_NAME");
    let path = path_without_the_binary()?;
    // A large payload closes the stdin-drain gap: a weakening that never reads stdin breaks the
    // pipe, so the test can tell a real drain (`cat >/dev/null`) from a silent skip.
    let payload = vec![b'x'; 1_500_000];
    let weakenings = [
        format!("{name} || exit 0"),
        format!("{name} || true"),
        name.to_string(),
        // Drains stdin but swallows the result into a green exit.
        "cat >/dev/null; exit 0".to_string(),
        // Mimics the run-failure message and exit code but never drains stdin.
        "echo 'did not run - nothing checked this write.' >&2; exit 1".to_string(),
    ];
    for command in &weakenings {
        // A broken pipe is itself a failure to satisfy the contract (exit 1 + "did not run"), so a
        // `run` error counts as "not contracted".
        let contracted = run(command, &path, &payload).is_ok_and(|output| {
            output.status.code() == Some(1)
                && String::from_utf8_lossy(&output.stderr).contains("did not run")
        });
        assert!(
            !contracted,
            "a weakened command satisfied the gatekeeper: {command}"
        );
    }
    Ok(())
}
