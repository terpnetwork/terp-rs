fn main() {
    if let Err(err) = terp_orch::gov::run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
