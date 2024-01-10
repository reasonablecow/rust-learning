fn main() -> anyhow::Result<()> {
    let _log_file_guard = server::init_logging_stdout_and_file()?;
    let address = server::Args::parse_to_address();
    server::run(&address)
}
