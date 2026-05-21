use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::{Parser, Subcommand};
use mzkana_core::{load_layout, InputEvent, Layout, MozcClient, MozcOutput, OutputAction, StateMachine};

#[derive(Parser)]
#[command(name = "mzkana", about = "MzKana layout tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a layout TOML file
    Validate {
        /// Path to layout TOML
        layout: PathBuf,
    },
    /// Run a key sequence through the state machine and print output actions
    Run {
        /// Path to layout TOML
        layout: PathBuf,
        /// Space-separated key sequence, e.g. "d k" or "f+j" for chord
        #[arg(short, long)]
        keys: String,
    },
    /// Run a key sequence through the state machine and feed output to Mozc,
    /// printing preedit/result after each action
    MozcRun {
        /// Path to layout TOML
        layout: PathBuf,
        /// Space-separated key sequence (same syntax as `run`)
        #[arg(short, long)]
        keys: String,
        /// Path to the Mozc server socket (default: ~/.mozc/session.sock)
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Print the JSON Schema for the layout file format
    Schema,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Validate { layout } => cmd_validate(&layout),
        Command::Run { layout, keys } => cmd_run(&layout, &keys),
        Command::MozcRun { layout, keys, socket } => cmd_mozc_run(&layout, &keys, socket.as_deref()),
        Command::Schema => cmd_schema(),
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn load_layout_or_exit(path: &Path) -> Layout {
    let src = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error reading {}: {e}", path.display());
        std::process::exit(1);
    });
    load_layout(&src).unwrap_or_else(|e| {
        eprintln!("error loading layout: {e}");
        std::process::exit(1);
    })
}

/// Parse and dispatch a single key token through the state machine.
///
/// Token syntax:
///   `"d"`   → key-down for d
///   `"d^"`  → key-up for d
///   `"f+j"` → chord: both keys down in sequence
fn process_token(sm: &mut StateMachine, token: &str, now: Instant) -> Result<Vec<OutputAction>, String> {
    if let Some(k) = token.strip_suffix('^') {
        if k.is_empty() {
            return Err(format!("invalid token {token:?}: empty key before '^'"));
        }
        if k.contains('+') {
            return Err(format!("invalid token {token:?}: '^' and '+' cannot be combined"));
        }
        return Ok(sm.process(InputEvent::up(k), now));
    }
    if token.contains('+') {
        if token.split('+').any(|s| s.is_empty()) {
            return Err(format!("invalid token {token:?}: empty segment in chord"));
        }
        return Ok(token
            .split('+')
            .flat_map(|k| sm.process(InputEvent::down(k), now))
            .collect());
    }
    if token.is_empty() {
        return Err("invalid token: empty key name".into());
    }
    Ok(sm.process(InputEvent::down(token), now))
}

// ── Subcommands ───────────────────────────────────────────────────────────────

fn cmd_validate(path: &Path) {
    let layout = load_layout_or_exit(path);
    println!(
        "OK: '{}' ({:?} mode, {} chord rules, {} direct rules)",
        layout.meta.name,
        layout.meta.mode,
        layout.chords.len(),
        layout.directs.len()
    );
}

fn cmd_run(path: &Path, keys: &str) {
    let mut sm = StateMachine::new(load_layout_or_exit(path));
    let now = Instant::now();

    for token in keys.split_whitespace() {
        let actions = process_token(&mut sm, token, now).unwrap_or_else(|e| {
            eprintln!("token error: {e}");
            std::process::exit(1);
        });
        for action in &actions {
            print_action(action);
        }
    }

    let tentative = sm.tentative_kana_string();
    if !tentative.is_empty() {
        println!("[preedit] {tentative}");
    }
}

fn cmd_mozc_run(path: &Path, keys: &str, socket: Option<&Path>) {
    let mut sm = StateMachine::new(load_layout_or_exit(path));
    let mut mozc = MozcClient::connect(socket).unwrap_or_else(|e| {
        eprintln!("failed to connect to Mozc: {e}");
        eprintln!(
            "(is mozc_server running? socket: {})",
            socket
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| mzkana_core::mozc::default_socket_path().display().to_string())
        );
        std::process::exit(1);
    });
    println!("Connected to Mozc (session {})", mozc.session_id());

    let now = Instant::now();
    for token in keys.split_whitespace() {
        let actions = process_token(&mut sm, token, now).unwrap_or_else(|e| {
            eprintln!("token error: {e}");
            std::process::exit(1);
        });
        for action in &actions {
            print_action(action);
            match dispatch_to_mozc(&mut mozc, action) {
                Ok(Some(out)) => {
                    print_mozc_output(&out);
                    if out.is_converting {
                        sm.notify_mozc_conversion();
                    } else {
                        sm.notify_mozc_composition();
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("[mozc error] {e}");
                    std::process::exit(1);
                }
            }
        }
    }

    let tentative = sm.tentative_kana_string();
    if !tentative.is_empty() {
        println!("[state-machine preedit] {tentative}");
    }
}

// ── Mozc dispatch ─────────────────────────────────────────────────────────────

fn dispatch_to_mozc(mozc: &mut MozcClient, action: &OutputAction) -> Result<Option<MozcOutput>, mzkana_core::MozcError> {
    match action {
        OutputAction::SendKana(s)      => mozc.send_kana(s).map(Some),
        OutputAction::Backspace        => mozc.send_backspace().map(Some),
        OutputAction::MozcSubmit       => mozc.submit().map(Some),
        OutputAction::SubmitAndCommit(s) => {
            let out = mozc.submit()?;
            println!("commit_direct({s})");
            Ok(Some(out))
        }
        // SendFunctionKey: unmapped keys (Muhenkan etc.) return Err(Protocol) which propagates.
        OutputAction::SendFunctionKey(name) => mozc.send_function_key(name).map(Some),
        OutputAction::CommitDirect(_) | OutputAction::Passthrough(_) => Ok(None),
    }
}

fn print_mozc_output(out: &MozcOutput) {
    if let Some(ref r) = out.result {
        println!("  → result: {r}");
    }
    if !out.preedit.is_empty() {
        println!("  → preedit: {}", out.preedit);
    }
    if out.is_converting {
        println!("  → [CONVERSION mode]");
    }
}

// ── Other commands ────────────────────────────────────────────────────────────

fn print_action(action: &OutputAction) {
    match action {
        OutputAction::SendKana(s)        => println!("send_kana({s})"),
        OutputAction::Backspace          => println!("backspace"),
        OutputAction::CommitDirect(s)    => println!("commit_direct({s})"),
        OutputAction::SubmitAndCommit(s) => println!("submit_and_commit({s})"),
        OutputAction::Passthrough(k)     => println!("passthrough({k})"),
        OutputAction::MozcSubmit         => println!("mozc_submit"),
        OutputAction::SendFunctionKey(k) => println!("send_function_key({k})"),
    }
}

fn cmd_schema() {
    let schema = schemars::schema_for!(mzkana_core::LayoutFile);
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
