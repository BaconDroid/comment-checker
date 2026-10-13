use crate::api::check_hook_input_from_reader;
use crate::engine::parse_options;

#[must_use]
pub fn run_from_env() -> i32 {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.iter().any(|argument| argument == "--version") {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return 0;
    }
    let options = parse_options(&arguments);
    let result = check_hook_input_from_reader(std::io::stdin().lock(), &options);
    eprint!("{}", result.message);
    result.exit_code
}
