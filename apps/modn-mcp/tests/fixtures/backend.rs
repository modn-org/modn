use std::io::{self, Write};
use std::path::PathBuf;

fn main() {
    let arguments: Vec<_> = std::env::args_os().collect();
    let index = arguments
        .iter()
        .position(|value| value == "--state-dir")
        .expect("state argument");
    let state = PathBuf::from(&arguments[index + 1]);
    let mode = std::fs::read_to_string(state.join("mode")).expect("fixture mode");
    match mode.as_str() {
        "large" => {
            io::stdout()
                .write_all(&vec![b'x'; 300_000])
                .expect("output");
        }
        "stderr" => {
            io::stderr()
                .write_all(&vec![b'x'; 300_000])
                .expect("output");
        }
        "sleep" => {
            std::fs::write(state.join("pid"), std::process::id().to_string()).expect("pid");
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
        "fail" => std::process::exit(7),
        _ => println!("invalid backend JSON"),
    }
}
