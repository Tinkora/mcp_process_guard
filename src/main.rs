use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use mcp_process_guard::{GuardError, GuardOptions, Outcome, run};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Output {
    Human,
    Json,
}

#[derive(Debug, Parser)]
#[command(
    name = "mcp-process-guard",
    version,
    about = "Bounded lifecycle check for one local MCP stdio server"
)]
struct Cli {
    #[arg(long)]
    no_handshake: bool,
    #[arg(long, default_value_t = 5_000, value_name = "MILLISECONDS", value_parser = parse_timeout)]
    handshake_timeout_ms: u64,
    #[arg(long, default_value_t = 2_000, value_name = "MILLISECONDS", value_parser = parse_timeout)]
    grace_ms: u64,
    #[arg(long, default_value_t = 2_000, value_name = "MILLISECONDS", value_parser = parse_cleanup_timeout)]
    cleanup_timeout_ms: u64,
    #[arg(long, default_value_t = 65_536, value_name = "BYTES", value_parser = parse_frame_size)]
    max_handshake_bytes: usize,
    #[arg(long, value_enum, default_value_t = Output::Human)]
    output: Output,
    #[arg(last = true, required = true, value_name = "COMMAND", num_args = 1..)]
    command: Vec<String>,
}

fn parse_bounded(value: &str, maximum: u64, label: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{label} must be an integer"))?;
    if (1..=maximum).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(format!("{label} must be between 1 and {maximum}"))
    }
}

fn parse_timeout(value: &str) -> Result<u64, String> {
    parse_bounded(value, 3_600_000, "timeout")
}

fn parse_cleanup_timeout(value: &str) -> Result<u64, String> {
    parse_bounded(value, 60_000, "cleanup timeout")
}

fn parse_frame_size(value: &str) -> Result<usize, String> {
    parse_bounded(value, 1_048_576, "handshake frame size").map(|parsed| parsed as usize)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let (command, args) = cli.command.split_first().expect("clap requires a command");
    let options = GuardOptions {
        command: command.clone(),
        args: args.to_vec(),
        handshake: !cli.no_handshake,
        handshake_timeout: Duration::from_millis(cli.handshake_timeout_ms),
        grace: Duration::from_millis(cli.grace_ms),
        cleanup_timeout: Duration::from_millis(cli.cleanup_timeout_ms),
        max_handshake_bytes: cli.max_handshake_bytes,
    };
    match run(&options) {
        Ok(report) => {
            match cli.output {
                Output::Json => println!(
                    "{}",
                    serde_json::to_string(&report).expect("serializable report")
                ),
                Output::Human => println!(
                    "outcome={:?} handshake={:?} elapsed_ms={} exit_code={} descendants_detected={} cleanup={:?}",
                    report.outcome,
                    report.handshake,
                    report.elapsed_ms,
                    report
                        .exit_code
                        .map_or_else(|| "none".into(), |code| code.to_string()),
                    report.descendants_detected,
                    report.cleanup,
                ),
            }
            match report.outcome {
                Outcome::Exited if report.exit_code == Some(0) => ExitCode::SUCCESS,
                Outcome::Exited => ExitCode::from(1),
                Outcome::TimedOut => ExitCode::from(3),
                Outcome::HandshakeFailed => ExitCode::from(4),
                Outcome::DescendantsSurvived => ExitCode::from(6),
                Outcome::CleanupFailed => ExitCode::from(7),
            }
        }
        Err(error) => {
            match cli.output {
                Output::Json => {
                    let outcome = match error {
                        GuardError::InvalidOptions => "input_failed",
                        GuardError::Spawn(_) => "spawn_failed",
                        GuardError::Ownership(_) => "ownership_failed",
                        GuardError::Wait(_) => "wait_failed",
                    };
                    println!(
                        r#"{{"outcome":"{outcome}","handshake":"skipped","elapsed_ms":0,"exit_code":null,"descendants_detected":false,"cleanup":"not_needed"}}"#
                    )
                }
                Output::Human => eprintln!("mcp-process-guard: {error}"),
            }
            ExitCode::from(5)
        }
    }
}
