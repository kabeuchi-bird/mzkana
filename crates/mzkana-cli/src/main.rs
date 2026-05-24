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

/// Find the owning PID of an abstract socket by matching its inode
/// (from /proc/net/unix) against /proc/*/fd/ symlinks.
#[cfg(target_os = "linux")]
fn find_socket_owner(socket_name: &str) -> Option<(u32, String)> {
    // Parse /proc/net/unix to find the inode for this socket name.
    // Also print the raw line for debugging.
    let unix_data = std::fs::read_to_string("/proc/net/unix").ok()?;
    let (target_inode, raw_line) = unix_data.lines().find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 8 && cols.last() == Some(&socket_name) {
            let inode: u64 = cols[6].parse().unwrap_or(0);
            Some((inode, line.to_string()))
        } else {
            None
        }
    })?;
    println!("    [/proc/net/unix] {raw_line}");
    if target_inode == 0 {
        println!("    → inode=0: 権限不足または LISTENING 状態のソケット");
        return None;
    }
    println!("    → inode={target_inode} でプロセス検索中...");

    // Walk /proc/*/fd/ — note: non-numeric entries (self, thread-self…) must be skipped,
    // so use `continue` instead of `?` to avoid aborting the whole search.
    let Ok(proc_dir) = std::fs::read_dir("/proc") else { return None };
    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else { continue };
        let fd_dir = format!("/proc/{pid}/fd");
        let Ok(fds) = std::fs::read_dir(&fd_dir) else { continue };
        for fd_entry in fds.flatten() {
            let Ok(target) = std::fs::read_link(fd_entry.path()) else { continue };
            // Socket symlinks look like "socket:[inode]"
            if target.to_string_lossy() == format!("socket:[{target_inode}]") {
                let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                return Some((pid, comm));
            }
        }
    }
    None
}

/// Show regular files open by a process (skip sockets/pipes).
#[cfg(target_os = "linux")]
fn show_process_files(pid: u32) {
    let fd_dir = format!("/proc/{pid}/fd");
    let Ok(fds) = std::fs::read_dir(&fd_dir) else {
        println!("    (fd ディレクトリ読み取り失敗: 権限不足)");
        return;
    };
    let mut found = false;
    for entry in fds.flatten() {
        if let Ok(target) = std::fs::read_link(entry.path()) {
            let t = target.to_string_lossy();
            if !t.starts_with("socket:[") && !t.starts_with("pipe:[") && !t.starts_with("anon_inode:") {
                println!("    fd {}: {t}", entry.file_name().to_string_lossy());
                found = true;
                // Show small file contents (potential key files)
                if let Ok(contents) = std::fs::read(&*t) {
                    if contents.len() <= 256 {
                        println!("      内容 ({} bytes): {:02x?}", contents.len(), contents);
                        if let Ok(s) = std::str::from_utf8(&contents) {
                            let s = s.trim_end_matches('\0');
                            if !s.is_empty() {
                                println!("      UTF-8: {s:?}");
                            }
                        }
                    }
                }
            }
        }
    }
    if !found {
        println!("    (通常ファイルなし — ソケット/パイプのみ)");
    }
}

fn cmd_diagnose_mozc(socket: Option<&Path>) {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    println!("=== Mozc IPC診断 ===");

    // 1. Socket discovery + owner
    let abs_name_opt = find_abstract_socket_name();
    let owner_pid: Option<u32> = match abs_name_opt {
        Some(ref name) => {
            print!("[1] abstract socket 発見: {name}");
            #[cfg(target_os = "linux")]
            let pid_opt = match find_socket_owner(name) {
                Some((pid, ref comm)) => { println!("  (所有: PID={pid} [{comm}])"); Some(pid) }
                None => { println!("  (所有プロセス不明)"); None }
            };
            #[cfg(not(target_os = "linux"))]
            let pid_opt: Option<u32> = { println!(); None };
            pid_opt
        }
        None => {
            println!("[1] abstract socket 未発見 (/proc/net/unix に .mozc. エントリなし)");
            None
        }
    };
    println!("    fallback パス: {}", mzkana_core::mozc::default_socket_path().display());

    // 1b. Show files open by mozc_server to find the IPC key file
    #[cfg(target_os = "linux")]
    if let Some(pid) = owner_pid {
        println!("[1b] mozc_server (PID={pid}) の開いているファイル:");
        show_process_files(pid);
    }

    // Helper: send bytes with size_t length prefix (8-byte NE)
    let send_msg = |stream: &mut UnixStream, data: &[u8], label: &str| -> bool {
        let len_prefix = data.len().to_ne_bytes();
        let mut frame = Vec::with_capacity(len_prefix.len() + data.len());
        frame.extend_from_slice(&len_prefix);
        frame.extend_from_slice(data);
        print!("{label} ({} bytes): {:02x?}", frame.len(), frame);
        match stream.write_all(&frame) {
            Ok(()) => { println!(" → OK"); true }
            Err(e) => { println!(" → 失敗: {e}"); false }
        }
    };

    // Helper: send bytes WITHOUT any length prefix (raw write)
    let send_raw = |stream: &mut UnixStream, data: &[u8], label: &str| -> bool {
        print!("{label} ({} bytes raw): {:02x?}", data.len(), data);
        match stream.write_all(data) {
            Ok(()) => { println!(" → OK"); true }
            Err(e) => { println!(" → 失敗: {e}"); false }
        }
    };

    // Helper: try a full handshake and report response.
    // key_framed: Some(bytes) = send with size_t framing; None = no key.
    // key_raw: Some(bytes) = send raw bytes (no framing).
    let try_handshake = |stream: &mut UnixStream, key_framed: Option<&[u8]>,
                         key_raw_opt: Option<&[u8]>, label: &str| {
        println!("  --- {label} ---");
        if let Some(k) = key_framed {
            if !send_msg(stream, k, "  key(framed)") { return; }
        }
        if let Some(k) = key_raw_opt {
            if !send_raw(stream, k, "  key(raw)") { return; }
        }

        // After sending key, wait briefly to see if server responds before CREATE_SESSION.
        // Bytes read here are stashed into `got` so the recv loop below sees them too.
        let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
        let mut got = Vec::new();
        {
            let mut peeked = [0u8; 32];
            match stream.read(&mut peeked) {
                Ok(0) => { println!("  [key後] EOF"); return; }
                Ok(n) => {
                    println!("  [key後] サーバーから {} bytes: {:02x?}", n, &peeked[..n]);
                    got.extend_from_slice(&peeked[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                       || e.kind() == std::io::ErrorKind::TimedOut => {
                    println!("  [key後] 応答なし (正常)");
                }
                Err(e) => { println!("  [key後] エラー: {e}"); return; }
            }
        }

        // Send CREATE_SESSION with size_t framing
        let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
        if !send_msg(stream, &[0x0a, 0x02, 0x08, 0x01], "  cmd") { return; }

        // Read response byte-by-byte to capture any partial data before EOF
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        loop {
            let mut b = [0u8; 1];
            match stream.read(&mut b) {
                Ok(0) => { println!("  recv: EOF after {} bytes: {:02x?}", got.len(), got); break; }
                Ok(_) => {
                    got.push(b[0]);
                    if got.len() >= 32 {
                        println!("  recv: {} bytes 受信 (続けて読み取ります): {:02x?}", got.len(), got);
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                       || e.kind() == std::io::ErrorKind::TimedOut => {
                    println!("  recv: タイムアウト (5秒), {} bytes received: {:02x?}", got.len(), got);
                    break;
                }
                Err(e) => { println!("  recv: エラー: {e}, {} bytes before: {:02x?}", got.len(), got); break; }
            }
        }
    };

    // Read IPC key from .session.ipc file (protobuf field 1, wire type 2).
    // Decodes LEB128 varints and scans past any leading fields to find field_number=1,wire=2.
    let ipc_file_key: Option<Vec<u8>> = {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let path = std::path::PathBuf::from(&home).join(".config/mozc/.session.ipc");
        std::fs::read(&path).ok().and_then(|data| {
            // Decode a varint (LEB128) from `buf[pos..]`; returns (value, new_pos) or None.
            fn read_varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
                let mut val: u64 = 0;
                let mut shift = 0u32;
                loop {
                    let b = *buf.get(pos)?;
                    pos += 1;
                    val |= ((b & 0x7f) as u64) << shift;
                    if b & 0x80 == 0 { return Some((val, pos)); }
                    shift += 7;
                    if shift >= 64 { return None; }
                }
            }
            // Walk the protobuf fields looking for field 1, wire type 2 (LEN).
            let mut pos = 0;
            while pos < data.len() {
                let (tag, next) = read_varint(&data, pos)?;
                pos = next;
                let field_number = tag >> 3;
                let wire_type = tag & 0x7;
                match wire_type {
                    2 => { // LEN: varint length followed by that many bytes
                        let (len, next) = read_varint(&data, pos)?;
                        pos = next;
                        let end = pos + len as usize;
                        if end > data.len() { return None; }
                        if field_number == 1 {
                            return Some(data[pos..end].to_vec());
                        }
                        pos = end;
                    }
                    0 => { // VARINT: skip
                        let (_, next) = read_varint(&data, pos)?;
                        pos = next;
                    }
                    1 => { pos += 8; } // 64-bit: skip
                    5 => { pos += 4; } // 32-bit: skip
                    _ => return None,
                }
            }
            None
        })
    };
    if let Some(ref k) = ipc_file_key {
        println!("[ipc_file_key] .session.ipc field1 ({} bytes): {:02x?}", k.len(),
                 &k[..k.len().min(40)]);
    }

    // 2. Attempt raw connections with different key formats
    println!("\n[2] 各種キー形式で接続を試行...");

    let connect_fresh = |label: &str| -> Option<UnixStream> {
        let s = if let Some(ref p) = socket.map(|x| x.to_path_buf()) {
            UnixStream::connect(p).ok()
        } else if abs_name_opt.is_some() {
            mzkana_core::mozc::connect_mozc_abstract().ok()
        } else {
            UnixStream::connect(mzkana_core::mozc::default_socket_path()).ok()
        };
        match s {
            Some(ref st) => {
                let _ = st.set_write_timeout(Some(Duration::from_secs(3)));
                println!("  [{label}] 接続成功");
            }
            None => println!("  [{label}] 接続失敗"),
        }
        s
    };

    // Variant A: no key
    if let Some(mut s) = connect_fresh("A: キーなし") {
        try_handshake(&mut s, None, None, "キーなし → CREATE_SESSION");
    }

    // Variant C: key = hash as ASCII (size_t framed)
    if let Some(ref abs_name) = abs_name_opt {
        let name = abs_name.strip_prefix('@').unwrap_or(abs_name.as_str());
        let hash_opt = name.strip_prefix("tmp/.mozc.").and_then(|s| s.strip_suffix(".session"));
        if let Some(hash) = hash_opt {
            let key_c = hash.as_bytes().to_vec();
            if let Some(mut s) = connect_fresh("C: hash ASCII キー (size_t framed)") {
                try_handshake(&mut s, Some(&key_c), None, &format!("C: hash {}B framed", key_c.len()));
            }

            // Variant D: hex-decoded raw bytes via the shared helper, size_t framed
            let key_d: Vec<u8> = mzkana_core::mozc::ipc_key_from_socket_name(abs_name)
                .unwrap_or_default();
            if !key_d.is_empty() {
                if let Some(mut s) = connect_fresh("D: 16B 生キー (size_t framed)") {
                    try_handshake(&mut s, Some(&key_d), None, &format!("D: raw {:02x?}", &key_d));
                }
            }

            // Variant E: hash as ASCII, NO framing (raw 32 bytes)
            if let Some(mut s) = connect_fresh("E: hash ASCII キー (フレームなし)") {
                try_handshake(&mut s, None, Some(key_c.as_slice()), "E: hash raw (no framing)");
            }

            // Variant F: hex-decoded raw bytes, NO framing
            if !key_d.is_empty() {
                if let Some(mut s) = connect_fresh("F: 16B 生キー (フレームなし)") {
                    try_handshake(&mut s, None, Some(&key_d), "F: raw bytes no framing");
                }
            }
        }
    }

    // Variant G: key from .session.ipc file, size_t framed
    if let Some(ref k) = ipc_file_key {
        if let Some(mut s) = connect_fresh("G: session.ipc field1 (size_t framed)") {
            try_handshake(&mut s, Some(k), None, &format!("G: ipc_file_key {}B framed", k.len()));
        }
        // Variant H: key from file, NO framing
        if let Some(mut s) = connect_fresh("H: session.ipc field1 (フレームなし)") {
            try_handshake(&mut s, None, Some(k), &format!("H: ipc_file_key {}B raw", k.len()));
        }
    }

    // Variants K: uint32 (4-byte LE) framing for both key and command.
    // This is the most likely correct protocol: the original attempt with uint32-only
    // command (no key) got EAGAIN (server responded but we read response as size_t).
    let send_u32 = |stream: &mut UnixStream, data: &[u8], label: &str| -> bool {
        let len_prefix = (data.len() as u32).to_le_bytes();
        let mut frame = Vec::with_capacity(4 + data.len());
        frame.extend_from_slice(&len_prefix);
        frame.extend_from_slice(data);
        print!("{label} (u32+{}B): {:02x?}", data.len(), frame);
        match stream.write_all(&frame) {
            Ok(()) => { println!(" → OK"); true }
            Err(e) => { println!(" → 失敗: {e}"); false }
        }
    };

    // Helper for u32-framed handshake (key optional, response read as u32)
    let try_u32_handshake = |stream: &mut UnixStream, key: Option<&[u8]>, label: &str| {
        println!("  --- {label} ---");
        if let Some(k) = key {
            if !send_u32(stream, k, "  key") { return; }
        }
        if !send_u32(stream, &[0x0a, 0x02, 0x08, 0x01], "  cmd") { return; }

        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        // Read response: u32 length prefix (4 bytes)
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf) {
            Ok(()) => {
                let resp_len = u32::from_le_bytes(len_buf) as usize;
                println!("  resp_len = {:02x?} → {} bytes", len_buf, resp_len);
                if resp_len < 65536 {
                    let mut body = vec![0u8; resp_len];
                    match stream.read_exact(&mut body) {
                        Ok(()) => println!("  resp body: {:02x?}", &body[..body.len().min(64)]),
                        Err(e) => println!("  resp body read failed: {e}"),
                    }
                } else {
                    println!("  resp_len 異常 ({resp_len})");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                   || e.kind() == std::io::ErrorKind::TimedOut => {
                println!("  recv: タイムアウト (サーバー応答なし)");
            }
            Err(e) => println!("  recv failed: {e}"),
        }
    };

    // K1: no key, u32 command
    if let Some(mut s) = connect_fresh("K1: キーなし u32フレーム") {
        try_u32_handshake(&mut s, None, "K1: no key, u32 cmd");
    }

    // K2: 32-byte ASCII key (u32 framed) + u32 command
    if let Some(ref abs_name) = abs_name_opt {
        let name = abs_name.strip_prefix('@').unwrap_or(abs_name.as_str());
        if let Some(hash) = name.strip_prefix("tmp/.mozc.").and_then(|s| s.strip_suffix(".session")) {
            let key_c = hash.as_bytes().to_vec();
            if let Some(mut s) = connect_fresh("K2: hash ASCII キー u32フレーム") {
                try_u32_handshake(&mut s, Some(&key_c), "K2: ascii key u32");
            }
        }
    }

    // K3: 32-byte key from .session.ipc (u32 framed) + u32 command
    if let Some(ref k) = ipc_file_key {
        if let Some(mut s) = connect_fresh("K3: session.ipc key u32フレーム") {
            try_u32_handshake(&mut s, Some(k), &format!("K3: ipc_file_key u32 ({}B)", k.len()));
        }
    }

    // Variants L: raw key bytes (NO length prefix) then u32-framed command.
    // Hypothesis: Mozc reads exactly kIPCKeySize(32) raw bytes for auth, then
    // switches to uint32-framed command loop.
    let send_raw = |stream: &mut UnixStream, data: &[u8], label: &str| -> bool {
        print!("{label} ({} bytes raw): {:02x?}", data.len(), data);
        match stream.write_all(data) {
            Ok(()) => { println!(" → OK"); true }
            Err(e) => { println!(" → 失敗: {e}"); false }
        }
    };

    let try_raw_key_u32_cmd = |stream: &mut UnixStream, key: &[u8], label: &str| {
        println!("  --- {label} ---");
        if !send_raw(stream, key, "  key (raw)") { return; }
        if !send_u32(stream, &[0x0a, 0x02, 0x08, 0x01], "  cmd") { return; }

        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf) {
            Ok(()) => {
                let resp_len = u32::from_le_bytes(len_buf) as usize;
                println!("  resp_len = {:02x?} → {} bytes", len_buf, resp_len);
                if resp_len < 65536 {
                    let mut body = vec![0u8; resp_len];
                    match stream.read_exact(&mut body) {
                        Ok(()) => println!("  resp body: {:02x?}", &body[..body.len().min(64)]),
                        Err(e) => println!("  resp body read failed: {e}"),
                    }
                } else {
                    println!("  resp_len 異常 ({resp_len})");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                   || e.kind() == std::io::ErrorKind::TimedOut => {
                println!("  recv: タイムアウト");
            }
            Err(e) => println!("  recv failed: {e}"),
        }
    };

    // L1: 32-byte ASCII hash (raw, no framing) + u32 command
    if let Some(ref abs_name) = abs_name_opt {
        let name = abs_name.strip_prefix('@').unwrap_or(abs_name.as_str());
        if let Some(hash) = name.strip_prefix("tmp/.mozc.").and_then(|s| s.strip_suffix(".session")) {
            let key_ascii = hash.as_bytes().to_vec();
            if let Some(mut s) = connect_fresh("L1: hash ASCII 32B 生 + u32コマンド") {
                try_raw_key_u32_cmd(&mut s, &key_ascii, &format!("L1: raw ASCII key ({}B)", key_ascii.len()));
            }
        }
    }

    // L2: 16-byte binary key (raw, no framing) + u32 command
    if let Some(ref abs_name) = abs_name_opt {
        let name = abs_name.strip_prefix('@').unwrap_or(abs_name.as_str());
        if let Some(hash) = name.strip_prefix("tmp/.mozc.").and_then(|s| s.strip_suffix(".session")) {
            if hash.len() % 2 == 0 {
                let key_bin: Vec<u8> = (0..hash.len()).step_by(2)
                    .filter_map(|i| hash.get(i..i+2).and_then(|s| u8::from_str_radix(s, 16).ok()))
                    .collect();
                if key_bin.len() == hash.len() / 2 {
                    if let Some(mut s) = connect_fresh("L2: binary 16B 生 + u32コマンド") {
                        try_raw_key_u32_cmd(&mut s, &key_bin, &format!("L2: raw binary key ({}B)", key_bin.len()));
                    }
                }
            }
        }
    }

    // L3: full socket path name (raw, no framing) + u32 command
    // Some Mozc builds use the full path as the IPC key.
    if let Some(ref abs_name) = abs_name_opt {
        let path_key = abs_name.strip_prefix('@').unwrap_or(abs_name.as_str()).as_bytes().to_vec();
        if let Some(mut s) = connect_fresh("L3: フルパス名 生 + u32コマンド") {
            try_raw_key_u32_cmd(&mut s, &path_key, &format!("L3: raw path key ({}B)", path_key.len()));
        }
    }

    // Variant M: read-first — does the server send a challenge/greeting immediately?
    if let Some(mut s) = connect_fresh("M: 接続後サーバー初回送信を待機") {
        println!("  --- M: 接続後に何も送らずサーバーの初回メッセージを最大3秒待機 ---");
        let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
        let mut buf = [0u8; 256];
        match s.read(&mut buf) {
            Ok(0) => println!("  M: サーバーが接続を即時クローズ"),
            Ok(n)  => {
                println!("  M: サーバーが先に {}B 送信: {:02x?}", n, &buf[..n]);
                println!("  M: ASCII: {:?}", String::from_utf8_lossy(&buf[..n]));
                // Try to read more
                let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
                let mut buf2 = [0u8; 256];
                if let Ok(n2) = s.read(&mut buf2) {
                    if n2 > 0 {
                        println!("  M (続き): {}B: {:02x?}", n2, &buf2[..n2]);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                   || e.kind() == std::io::ErrorKind::TimedOut => {
                println!("  M: 3秒待機してもサーバーから送信なし → クライアント先送りプロトコル");
            }
            Err(e) => println!("  M: read error: {e}"),
        }
    }
}

fn cmd_schema() {
    let schema = schemars::schema_for!(mzkana_core::LayoutFile);
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
