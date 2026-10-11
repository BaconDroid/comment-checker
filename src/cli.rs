use crate::api::check_hook_input_from_reader;

#[must_use]
pub fn run_from_env() -> i32 {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = check_hook_input_from_reader(std::io::stdin().lock(), &parse_prompt(&arguments));
    eprint!("{}", result.message);
    result.exit_code
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
