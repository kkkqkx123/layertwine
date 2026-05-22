fn main() {
    let exit_code = stratum::api::cli::run();
    std::process::exit(exit_code);
}