//! `ed-corai-cli`: ED Corai's MCP server and command line.
//!
//!   ed-corai-cli mcp                         serve Claude (or any MCP app) over stdio
//!   ed-corai-cli evaluate <build|ship>       full stats of a build
//!   ed-corai-cli optimize <build|ship> <metric> [--constraints JSON] [--lock a,b] [--effort quick]
//!   ed-corai-cli tool <name> [JSON]          any tool with JSON arguments
//!   ed-corai-cli version | check-update

// This is a command-line program: printing results is its job.
#![allow(clippy::print_stdout)]

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use corai_service::dispatch::call;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "ed-corai-cli",
    version,
    about = "Elite Dangerous ship builds worked out by the numbers"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the tools over MCP (stdin/stdout) for Claude and other AI apps.
    Mcp {
        /// Do not look for a newer release at start-up.
        #[arg(long)]
        no_update_check: bool,
    },
    /// Full stats of a build: a ship id or name, a Coriolis or EDSY link, SLEF, a journal file.
    Evaluate {
        build: String,
        /// Settings as JSON, e.g. '{"damagePreset":"pve","sysPips":4}'.
        #[arg(long)]
        context: Option<String>,
        /// Slot changes as a JSON array.
        #[arg(long)]
        changes: Option<String>,
    },
    /// Best outfitting of one ship for an objective metric.
    Optimize {
        build: String,
        metric: String,
        /// Constraints as a JSON array, e.g. '[{"metric":"ladenRange","min":20}]'.
        #[arg(long)]
        constraints: Option<String>,
        /// Slots to keep, e.g. hardpoint,standard:fsd.
        #[arg(long, value_delimiter = ',')]
        lock: Vec<String>,
        /// quick, normal or thorough.
        #[arg(long)]
        effort: Option<String>,
        #[arg(long)]
        context: Option<String>,
    },
    /// Calls any tool with JSON arguments.
    Tool { name: String, args: Option<String> },
    /// Version and the Coriolis code and data this build contains.
    Version,
    /// Looks for a newer release on GitHub.
    CheckUpdate,
    /// Downloads a release (default: the latest) and replaces this program.
    Update {
        /// Release tag, e.g. v0.2.0.
        #[arg(long)]
        version: Option<String>,
    },
}

/// A build argument: a file's contents when it names a file, else the text itself.
fn build_text(arg: &str) -> String {
    std::fs::read_to_string(arg).unwrap_or_else(|_| arg.to_string())
}

/// Tool arguments for a build or ship argument.
fn build_or_ship(arg: &str) -> (&'static str, String) {
    let text = build_text(arg);
    let looks_like_ship = !text.contains(['/', '{', '[', ':', '\n']) && !text.starts_with("http");
    if looks_like_ship {
        ("ship", text)
    } else {
        ("build", text)
    }
}

fn parse_json(label: &str, text: Option<&str>) -> Result<Option<Value>, String> {
    text.map(|t| serde_json::from_str(t).map_err(|e| format!("{label} is not valid JSON: {e}")))
        .transpose()
}

fn run(cli: Cli) -> Result<Value, String> {
    let mut args = serde_json::Map::new();
    let tool = match cli.command {
        Command::Mcp { .. } => unreachable!("handled in main"),
        Command::Version => return Ok(corai_mcp::update::build_info()),
        Command::CheckUpdate => return Ok(corai_mcp::update::check_for_updates()),
        Command::Update { version } => {
            return corai_mcp::update::install_update(version.as_deref());
        }
        Command::Tool { name, args: a } => {
            let v = parse_json("args", a.as_deref())?.unwrap_or_else(|| json!({}));
            return corai_mcp::run_tool_text(&name, &v);
        }
        Command::Evaluate {
            build,
            context,
            changes,
        } => {
            let (k, v) = build_or_ship(&build);
            args.insert(k.into(), v.into());
            if let Some(c) = parse_json("--context", context.as_deref())? {
                args.insert("context".into(), c);
            }
            if let Some(c) = parse_json("--changes", changes.as_deref())? {
                args.insert("changes".into(), c);
            }
            "evaluate_build"
        }
        Command::Optimize {
            build,
            metric,
            constraints,
            lock,
            effort,
            context,
        } => {
            let (k, v) = build_or_ship(&build);
            args.insert(k.into(), v.into());
            args.insert("objective".into(), json!({ "metric": metric }));
            if let Some(c) = parse_json("--constraints", constraints.as_deref())? {
                args.insert("constraints".into(), c);
            }
            if !lock.is_empty() {
                args.insert("lock".into(), lock.into());
            }
            if let Some(e) = effort {
                args.insert("effort".into(), e.into());
            }
            if let Some(c) = parse_json("--context", context.as_deref())? {
                args.insert("context".into(), c);
            }
            "optimize_build"
        }
    };
    call(tool, &Value::Object(args)).map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    corai_mcp::update::cleanup_previous();
    if let Command::Mcp { no_update_check } = cli.command {
        let check = !no_update_check && std::env::var("CORAI_UPDATE_CHECK").as_deref() != Ok("0");
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("Cannot start: {e}");
                return ExitCode::FAILURE;
            }
        };
        return match rt.block_on(corai_mcp::serve_stdio(check)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("MCP server stopped: {e}");
                ExitCode::FAILURE
            }
        };
    }
    match run(cli) {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
