use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::{Parser, Subcommand};
use mzkana_core::{load_layout, mozc::find_abstract_socket_name, InputEvent, Layout, MozcClient, MozcOutput, OutputAction, StateMachine};

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
    /// Diagnose the Mozc IPC connection (socket discovery, raw byte exchange)
    DiagnoseMozc {
        /// Path to the Mozc server socket (overrides auto-discovery)
        #[arg(long)]
        socket: Option<PathBuf>,
    },
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
        Command::DiagnoseMozc { socket } => cmd_diagnose_mozc(socket.as_deref()),
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
        OutputAction::SendModifiedKey { key, mods } => {
            // CLI has no application to forward to; log and attempt Mozc routing.
            println!("  modified_key: mods={mods:#04b} key={key}");
            mozc.send_modified_key(key, *mods).map(Some)
        }
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
        OutputAction::SendModifiedKey { key, mods } => println!("modified_key(mods={mods:#04b}, key={key})"),
    }
}

fn cmd_diagnose_mozc(socket: Option<&Path>) {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    // 1. Show socket discovery result
    println!("=== Mozc IPC診断 ===");
    match find_abstract_socket_name() {
        Some(ref name) => println!("[1] abstract socket 発見: {name}"),
        None => println!("[1] abstract socket 未発見 (/proc/net/unix に .mozc. エントリなし)"),
    }
    println!("    fallback パス: {}", mzkana_core::mozc::default_socket_path().display());

    // 2. Attempt raw connection
    let stream_result: std::io::Result<UnixStream> = if let Some(p) = socket {
        println!("[2] 指定パスで接続試行: {}", p.display());
        UnixStream::connect(p)
    } else {
        match find_abstract_socket_name() {
            Some(ref abs) => {
                println!("[2] abstract socket に接続試行: {abs}");
                mzkana_core::mozc::connect_mozc_abstract()
            }
            None => {
                println!("[2] fallback パスで接続試行");
                UnixStream::connect(mzkana_core::mozc::default_socket_path())
            }
        }
    };

    let mut stream = match stream_result {
        Ok(s) => { println!("[2] 接続成功"); s }
        Err(e) => { println!("[2] 接続失敗: {e}"); return; }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    // 3. Check if server sends a greeting before we write anything
    println!("[3] サーバーからの先行送信チェック (200ms待機)...");
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut greeting = [0u8; 64];
    match stream.read(&mut greeting) {
        Ok(0) => println!("    → EOF (サーバーが即時クローズ)"),
        Ok(n) => println!("    → {n}バイト受信 (greeting): {:02x?}", &greeting[..n]),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
               || e.kind() == std::io::ErrorKind::TimedOut => {
            println!("    → タイムアウト (先行送信なし、正常)");
        }
        Err(e) => println!("    → エラー: {e}"),
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));

    // 4. Send CREATE_SESSION and show raw bytes
    // Command { input { type: CREATE_SESSION(1) } }
    // Encoded: [0a 02 08 01]  frame: [size_t NE len][proto bytes]
    let proto: &[u8] = &[0x0a, 0x02, 0x08, 0x01];
    let len_prefix = proto.len().to_ne_bytes();
    let mut frame = Vec::with_capacity(len_prefix.len() + proto.len());
    frame.extend_from_slice(&len_prefix);
    frame.extend_from_slice(proto);
    println!("[4] CREATE_SESSION 送信: {:02x?}", frame);
    match stream.write_all(&frame) {
        Ok(()) => println!("    → 送信成功"),
        Err(e) => { println!("    → 送信失敗: {e}"); return; }
    }

    // 5. Read response length (size_t = 8 bytes on 64-bit)
    println!("[5] レスポンス長 ({}バイト) 読み取り...", std::mem::size_of::<usize>());
    let mut len_buf = [0u8; std::mem::size_of::<usize>()];
    match stream.read_exact(&mut len_buf) {
        Ok(()) => {
            let resp_len = usize::from_ne_bytes(len_buf);
            println!("    → 長さバイト: {:02x?} → {resp_len} バイト", len_buf);

            // 6. Read response body
            if resp_len < 65536 {
                let mut resp = vec![0u8; resp_len];
                match stream.read_exact(&mut resp) {
                    Ok(()) => println!("[6] レスポンス本体 ({resp_len}B): {:02x?}", &resp[..resp_len.min(64)]),
                    Err(e) => println!("[6] レスポンス本体 読み取り失敗: {e}"),
                }
            } else {
                println!("[6] 長さが異常 ({resp_len}), big-endian かも");
            }
        }
        Err(e) => println!("    → 読み取り失敗: {e}"),
    }
}

fn cmd_schema() {
    let schema = schemars::schema_for!(mzkana_core::LayoutFile);
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
