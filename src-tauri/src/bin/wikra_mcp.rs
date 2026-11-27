#[cfg(windows)]
fn main() -> std::process::ExitCode {
    wikra::mcp::main_stdio()
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("Wikra MCP requires Windows");
    std::process::ExitCode::FAILURE
}
