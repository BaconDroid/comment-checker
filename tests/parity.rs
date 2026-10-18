//! Cross-repo hook-contract conformance suite. This file is byte-identical in `comment-checker`
//! and `test-checker`, and runs the cases from the shared `parity/vectors.json`.

use std::io::Write;
use std::process::{Command, Output, Stdio};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Vectors {
    prefix: String,
    placeholder: String,
    element_open: String,
    clean: String,
    trigger: String,
    cases: Vec<Case>,
}

struct Case {
    name: String,
    input: String,
    exit: i32,
    stderr_contains: Option<String>,
}

fn vectors() -> Result<Vectors, Box<dyn std::error::Error>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/parity/vectors.json");
    let raw = std::fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&raw)?;
    let section = json
        .get(env!("CARGO_PKG_NAME"))
        .ok_or("missing vectors section for this crate")?;
    let field = |key: &str| -> Result<String, Box<dyn std::error::Error>> {
        Ok(section
            .get(key)
            .and_then(serde_json::Value::as_str)
            .ok_or("missing vector field")?
            .to_string())
    };
    let cases = parse_cases(section)?;
    Ok(Vectors {
        prefix: field("prefix")?,
        placeholder: field("placeholder")?,
        element_open: field("element_open")?,
        clean: field("clean")?,
        trigger: field("trigger")?,
        cases,
    })
}

/// Parse the `cases` array, failing loudly on a malformed entry or a missing/empty list, so the
/// contract test can never pass by silently running zero cases.
fn parse_cases(section: &serde_json::Value) -> Result<Vec<Case>, Box<dyn std::error::Error>> {
    let array = section
        .get("cases")
        .and_then(serde_json::Value::as_array)
        .ok_or("vectors section has no `cases` array")?;
    if array.is_empty() {
        return Err("vectors section `cases` array is empty".into());
    }
    let mut cases = Vec::with_capacity(array.len());
    for case in array {
        let name = case
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or("a case is missing `name`")?;
        let input = case
            .get("input")
            .and_then(serde_json::Value::as_str)
            .ok_or("a case is missing `input`")?;
        let exit = case
            .get("exit")
            .and_then(serde_json::Value::as_i64)
            .ok_or("a case is missing `exit`")?;
        let exit = i32::try_from(exit).map_err(|_| "a case `exit` is out of i32 range")?;
        // Optional, but a present non-string value is a malformed vector, not an absent field.
        let stderr_contains = match case.get("stderr_contains") {
            None => None,
            Some(serde_json::Value::String(value)) => Some(value.clone()),
            Some(_) => return Err("a case `stderr_contains` is not a string".into()),
        };
        cases.push(Case {
            name: name.to_string(),
            input: input.to_string(),
            exit,
            stderr_contains,
        });
    }
    Ok(cases)
}

fn run(input: &str, args: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
    let binary = std::env::var(format!("CARGO_BIN_EXE_{}", env!("CARGO_PKG_NAME")))?;
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .as_mut()
        .ok_or("stdin handle unavailable")?
        .write_all(input.as_bytes())?;
    Ok(child.wait_with_output()?)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn clean_input_exits_zero_with_a_prefixed_success() -> TestResult {
    let v = vectors()?;
    let output = run(&v.clean, &[])?;

    assert_eq!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains(&format!("{} Success", v.prefix)),
        "stderr: {}",
        stderr(&output)
    );
    Ok(())
}

#[test]
fn triggering_input_exits_two_with_xml() -> TestResult {
    let v = vectors()?;
    let output = run(&v.trigger, &[])?;
    let text = stderr(&output);

    assert_eq!(output.status.code(), Some(2));
    assert!(text.contains(&v.element_open), "stderr: {text}");
    Ok(())
}

#[test]
fn custom_prompt_without_placeholder_is_exact() -> TestResult {
    let v = vectors()?;
    let output = run(&v.trigger, &["--prompt", "CUSTOM"])?;

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stderr(&output), "CUSTOM");
    Ok(())
}

#[test]
fn custom_prompt_with_placeholder_embeds_the_xml() -> TestResult {
    let v = vectors()?;
    let prompt = format!("PRE {} POST", v.placeholder);
    let output = run(&v.trigger, &["--prompt", &prompt])?;
    let text = stderr(&output);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        text.contains("PRE") && text.contains(&v.element_open) && text.contains("POST"),
        "stderr: {text}"
    );
    Ok(())
}

#[test]
fn empty_stdin_fails_open_to_exit_zero() -> TestResult {
    let v = vectors()?;
    let output = run("", &[])?;

    assert_eq!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains(&format!("{} Skipping:", v.prefix)),
        "stderr: {}",
        stderr(&output)
    );
    Ok(())
}

#[test]
fn invalid_json_fails_open_to_exit_zero() -> TestResult {
    let output = run("not json", &[])?;

    assert_eq!(output.status.code(), Some(0));
    assert!(stderr(&output).contains("Invalid input format"));
    Ok(())
}

#[test]
fn unknown_positional_arguments_are_ignored() -> TestResult {
    let v = vectors()?;
    let output = run(&v.clean, &["some", "unexpected", "args"])?;

    assert_eq!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains(&format!("{} Success", v.prefix)),
        "stderr: {}",
        stderr(&output)
    );
    Ok(())
}

#[test]
fn declared_contract_cases_exit_as_specified() -> TestResult {
    let v = vectors()?;
    for case in &v.cases {
        let output = run(&case.input, &[])?;
        assert_eq!(
            output.status.code(),
            Some(case.exit),
            "case {}: stderr {}",
            case.name,
            stderr(&output)
        );
        if let Some(needle) = &case.stderr_contains {
            let text = stderr(&output);
            assert!(
                text.contains(needle),
                "case {}: expected stderr to contain {needle:?}, got: {text}",
                case.name
            );
        }
    }
    Ok(())
}
