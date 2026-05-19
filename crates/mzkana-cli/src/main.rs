use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use mzkana_core::{load_layout, InputEvent, OutputAction, StateMachine};

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
    }
}

fn cmd_schema() {
    let schema = schemars::schema_for!(mzkana_core::LayoutFile);
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
