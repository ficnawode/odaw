use std::path::PathBuf;

fn main() {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    if let Err(error) = odaw_ui::run(path) {
        eprintln!("odaw-app: {error}");
        std::process::exit(1);
    }
}
