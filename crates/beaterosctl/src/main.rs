use std::process::ExitCode;

fn main() -> ExitCode {
    match beaterosctl::run(std::env::args()) {
        Ok(output) => {
            if !output.is_empty() {
                println!("{output}");
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("beaterosctl error: {err}");
            ExitCode::FAILURE
        }
    }
}
