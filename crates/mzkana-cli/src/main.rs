use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::{Parser, Subcommand};
use mzkana_core::{load_layout, InputEvent, MozcClient, MozcOutput, OutputAction, StateMachine};

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
        /// Path to the Mozc server socket (default: ~/.mozc/server.sock)
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
        Command::MozcRun { layout, keys, socket } => {
            cmd_mozc_run(&layout, &keys, socket.as_deref())
        }
        Command::Schema => cmd_schema(),
    }
}

fn cmd_validate(path: &PathBuf) {
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    match load_layout(&src) {
        Ok(layout) => {
            println!(
                "OK: '{}' ({:?} mode, {} chord rules, {} direct rules)",
                layout.meta.name,
                layout.meta.mode,
                layout.chords.len(),
                layout.directs.len()
            );
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_run(path: &PathBuf, keys: &str) {
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    let layout = match load_layout(&src) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error loading layout: {e}");
            std::process::exit(1);
        }
    };

    let mut sm = StateMachine::new(layout);
    let now = Instant::now();

    // Parse key sequence tokens:
    //   "d k"   → two separate key-downs
    //   "f+j"   → chord: f down, j down (within chord window)
    //   "d^"    → key up for d
    for token in keys.split_whitespace() {
        let actions = if token.contains('+') {
            // chord: press all keys in quick succession
            let chord_keys: Vec<&str> = token.split('+').collect();
            let mut all_actions = Vec::new();
            for k in &chord_keys {
                let ev = InputEvent::down(*k);
                all_actions.extend(sm.process(ev, now));
            }
            all_actions
        } else if let Some(k) = token.strip_suffix('^') {
            sm.process(InputEvent::up(k), now)
        } else {
            sm.process(InputEvent::down(token), now)
        };

        for action in &actions {
            print_action(action);
        }
    }

    // Print tentative buffer state
    let tentative = sm.tentative_kana_string();
    if !tentative.is_empty() {
        println!("[preedit] {tentative}");
    }
}

fn print_action(action: &OutputAction) {
    match action {
        OutputAction::SendKana(s) => println!("send_kana({s})"),
        OutputAction::Backspace => println!("backspace"),
        OutputAction::CommitDirect(s) => println!("commit_direct({s})"),
        OutputAction::SubmitAndCommit(s) => println!("submit_and_commit({s})"),
        OutputAction::Passthrough(k) => println!("passthrough({k})"),
        OutputAction::MozcSubmit => println!("mozc_submit"),
        OutputAction::SendFunctionKey(k) => println!("send_function_key({k})"),
    }
}

fn cmd_mozc_run(path: &PathBuf, keys: &str, socket: Option<&Path>) {
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    let layout = match load_layout(&src) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error loading layout: {e}");
            std::process::exit(1);
        }
    };
    let mut mozc = match MozcClient::connect(socket) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to connect to Mozc: {e}");
            eprintln!("(is mozc_server running? socket: {})",
                socket.map(|p| p.display().to_string())
                    .unwrap_or_else(|| mzkana_core::mozc::default_socket_path().display().to_string()));
            std::process::exit(1);
        }
    };
    println!("Connected to Mozc (session {})", mozc.session_id.unwrap_or(0));

    let mut sm = StateMachine::new(layout);
    let now = Instant::now();

    for token in keys.split_whitespace() {
        let actions = if token.contains('+') {
            let chord_keys: Vec<&str> = token.split('+').collect();
            let mut all_actions = Vec::new();
            for k in &chord_keys {
                all_actions.extend(sm.process(InputEvent::down(*k), now));
            }
            all_actions
        } else if let Some(k) = token.strip_suffix('^') {
            sm.process(InputEvent::up(k), now)
        } else {
            sm.process(InputEvent::down(token), now)
        };

        for action in &actions {
            print_action(action);
            let mozc_out = dispatch_to_mozc(&mut mozc, action);
            if let Some(out) = mozc_out {
                print_mozc_output(&out);
                // Keep state machine in sync with Mozc mode
                if out.is_converting {
                    sm.notify_mozc_conversion();
                } else {
                    sm.notify_mozc_composition();
                }
            }
        }
    }

    let tentative = sm.tentative_kana_string();
    if !tentative.is_empty() {
        println!("[state-machine preedit] {tentative}");
    }
}

/// Translate a single `OutputAction` into a Mozc call and return the response.
fn dispatch_to_mozc(mozc: &mut MozcClient, action: &OutputAction) -> Option<MozcOutput> {
    match action {
        OutputAction::SendKana(s) => {
            match mozc.send_kana(s) {
                Ok(out) => Some(out),
                Err(e) => { eprintln!("[mozc error] {e}"); None }
            }
        }
        OutputAction::Backspace => {
            match mozc.send_backspace() {
                Ok(out) => Some(out),
                Err(e) => { eprintln!("[mozc error] {e}"); None }
            }
        }
        OutputAction::MozcSubmit => {
            match mozc.submit() {
                Ok(out) => Some(out),
                Err(e) => { eprintln!("[mozc error] {e}"); None }
            }
        }
        // CommitDirect / SubmitAndCommit: kanchoku paths — also submit preedit
        OutputAction::SubmitAndCommit(s) => {
            let _ = mozc.submit();
            println!("commit_direct({s})");
            None
        }
        OutputAction::CommitDirect(_) | OutputAction::Passthrough(_) | OutputAction::SendFunctionKey(_) => None,
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

fn cmd_schema() {
    let schema = schemars::schema_for!(mzkana_core::LayoutFile);
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
