fn main() {
    client::run().unwrap_or_else(|e| {
        eprintln!("Client crashed, because \"{e}\"");
        std::process::exit(1);
    })
}
