use crate::api::{EXIT_PASS, check_hook_input};

#[must_use]
pub fn run_from_env() -> i32 {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut input = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).is_err() {
        eprintln!("[check-comments] Skipping: Failed to read stdin");
        return EXIT_PASS;
    }
    run_with_input(&input, &parse_prompt(&arguments))
}

fn parse_prompt(arguments: &[String]) -> String {
    let mut prompt = String::new();
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        if let Some(value) = argument.strip_prefix("--prompt=") {
            value.clone_into(&mut prompt);
        } else if argument == "--prompt" {
            if let Some(value) = arguments.get(index + 1) {
                prompt.clone_from(value);
                index += 1;
            }
        }
        index += 1;
    }
    prompt
}

#[must_use]
pub fn run_with_input(input: &str, custom_prompt: &str) -> i32 {
    let result = check_hook_input(input, custom_prompt);
    eprint!("{}", result.message);
    result.exit_code
}
