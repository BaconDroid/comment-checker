use crate::api::check_hook_input_from_reader;
use crate::engine::parse_prompt;

#[must_use]
pub fn run_from_env() -> i32 {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = check_hook_input_from_reader(std::io::stdin().lock(), &parse_prompt(&arguments));
    eprint!("{}", result.message);
    result.exit_code
}
