fn main() -> std::process::ExitCode {
    sleipnir_agentctl::run_cli(std::env::args().skip(1))
}
