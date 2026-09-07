use anyhow::Result;
use tracing::info;

/// What `main` should do, decided purely from argv.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    PrintVersion,
    PrintHelp,
    RunDaemon,
}

const USAGE: &str = "aurelius: run with no arguments to start the MCP daemon on stdio\n  -V, --version   print the crate version and exit\n  -h, --help      print this help and exit";

/// Decide what to do based on the CLI arguments (excluding argv[0]).
///
/// Unknown arguments are ignored and fall through to running the daemon,
/// matching the pre-existing behavior of this binary.
fn parse_action<I: IntoIterator<Item = String>>(args: I) -> Action {
    for arg in args {
        match arg.as_str() {
            "--version" | "-V" => return Action::PrintVersion,
            "--help" | "-h" => return Action::PrintHelp,
            _ => {}
        }
    }
    Action::RunDaemon
}

#[tokio::main]
async fn main() -> Result<()> {
    // Handle --version/--help before touching tracing: once fmt::init runs,
    // log lines on stderr would interleave with the answer.
    match parse_action(std::env::args().skip(1)) {
        Action::PrintVersion => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Action::PrintHelp => {
            println!("{USAGE}");
            return Ok(());
        }
        Action::RunDaemon => {}
    }

    tracing_subscriber::fmt::init();
    info!("Aurelius daemon starting");
    aurelius::mcp::serve().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn long_version_flag_prints_version() {
        assert_eq!(parse_action(args(&["--version"])), Action::PrintVersion);
    }

    #[test]
    fn short_version_flag_prints_version() {
        assert_eq!(parse_action(args(&["-V"])), Action::PrintVersion);
    }

    #[test]
    fn long_help_flag_prints_help() {
        assert_eq!(parse_action(args(&["--help"])), Action::PrintHelp);
    }

    #[test]
    fn short_help_flag_prints_help() {
        assert_eq!(parse_action(args(&["-h"])), Action::PrintHelp);
    }

    #[test]
    fn unknown_flag_runs_daemon() {
        assert_eq!(parse_action(args(&["--bogus"])), Action::RunDaemon);
    }

    #[test]
    fn empty_args_run_daemon() {
        assert_eq!(parse_action(args(&[])), Action::RunDaemon);
    }
}
