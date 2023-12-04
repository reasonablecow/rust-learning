fn main() {
    let _log_file_guard = server::init_logging_stdout_and_file().unwrap_or_else(|e| {
        eprintln!("Logging initialization failed, error \"{e}\"");
        std::process::exit(1);
    });
    let address = server::Args::parse_to_address();

    server::run(&address).unwrap_or_else(|e| {
        eprintln!("Server crashed, because \"{e}\"");
        std::process::exit(1);
    })
}
