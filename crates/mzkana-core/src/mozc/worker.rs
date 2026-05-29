//! Off-thread Mozc IPC with a hard response timeout (C5).
//!
//! `MozcClient` performs blocking Unix-socket round-trips.  Calling it directly
//! from the fcitx5 main event loop means a slow or hung `mozc_server` freezes
//! desktop input for up to the socket timeout, multiplied by the number of
//! round-trips a single keystroke triggers (a chord rewrite is BackSpace×N +
//! SendKana).  `MozcWorker` moves the client onto a dedicated thread and gives
//! the caller a **150 ms hard timeout**: if the worker does not answer in time,
//! the connection is declared dead and the caller proceeds without it (the UI
//! never blocks).
//!
//! A whole keystroke's worth of operations is sent as a single `Batch` so the
//! channel round-trip and the timeout budget are paid once, not per operation.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::client::{MozcClient, MozcError};
use super::MozcOutput;

/// Hard timeout for a batch round-trip.  Past this the worker is considered dead
/// and the UI proceeds without waiting further.
pub const WORKER_TIMEOUT: Duration = Duration::from_millis(150);

/// A single Mozc operation, mirroring the `MozcClient` methods the engine uses.
#[derive(Debug, Clone)]
pub enum Op {
    SendKana(String),
    Backspace,
    Submit,
    Revert,
    SendFunctionKey(String),
    SendModified { key: String, mods: u8 },
}

/// Anything that can execute the Mozc operations a batch contains.
///
/// `MozcClient` implements this; tests use a mock so the worker/batch logic can
/// be exercised without a running `mozc_server`.
pub trait MozcSession {
    fn send_kana(&self, kana: &str) -> Result<MozcOutput, MozcError>;
    fn send_backspace(&self) -> Result<MozcOutput, MozcError>;
    fn submit(&self) -> Result<MozcOutput, MozcError>;
    fn revert(&self) -> Result<MozcOutput, MozcError>;
    fn send_function_key(&self, name: &str) -> Result<MozcOutput, MozcError>;
    fn send_modified_key(&self, key: &str, mods: u8) -> Result<MozcOutput, MozcError>;
}

impl MozcSession for MozcClient {
    fn send_kana(&self, kana: &str) -> Result<MozcOutput, MozcError> { MozcClient::send_kana(self, kana) }
    fn send_backspace(&self) -> Result<MozcOutput, MozcError> { MozcClient::send_backspace(self) }
    fn submit(&self) -> Result<MozcOutput, MozcError> { MozcClient::submit(self) }
    fn revert(&self) -> Result<MozcOutput, MozcError> { MozcClient::revert(self) }
    fn send_function_key(&self, name: &str) -> Result<MozcOutput, MozcError> { MozcClient::send_function_key(self, name) }
    fn send_modified_key(&self, key: &str, mods: u8) -> Result<MozcOutput, MozcError> { MozcClient::send_modified_key(self, key, mods) }
}

/// Execute one op against a session.  Free function so it can be unit-tested
/// against a mock `MozcSession`.
pub fn apply_op(session: &dyn MozcSession, op: &Op) -> Result<MozcOutput, MozcError> {
    match op {
        Op::SendKana(s) => session.send_kana(s),
        Op::Backspace => session.send_backspace(),
        Op::Submit => session.submit(),
        Op::Revert => session.revert(),
        Op::SendFunctionKey(name) => session.send_function_key(name),
        Op::SendModified { key, mods } => session.send_modified_key(key, *mods),
    }
}

/// Result of a batch: one entry per submitted op, in order.
pub type BatchResult = Vec<Result<MozcOutput, MozcError>>;

enum Request {
    Batch(Vec<Op>),
    Shutdown,
}

/// Error from a worker round-trip.
#[derive(Debug)]
pub enum WorkerError {
    /// The worker did not answer within `WORKER_TIMEOUT`, or the thread is gone.
    /// The worker should be discarded and recreated.
    Dead,
}

/// A `MozcClient` running on its own thread, accessed with a hard timeout.
pub struct MozcWorker {
    req_tx: Sender<Request>,
    resp_rx: Receiver<BatchResult>,
    handle: Option<JoinHandle<()>>,
    /// Once a round-trip times out the channel state is ambiguous, so the worker
    /// is sticky-dead: every later call fails fast until it is recreated.
    dead: bool,
    session_id: u64,
}

impl MozcWorker {
    /// Connect a `MozcClient` (this blocks, as connecting may spawn `mozc_server`)
    /// and hand it to a dedicated worker thread.  Returns once the session is live.
    pub fn connect(socket_path: Option<&std::path::Path>) -> Result<Self, MozcError> {
        let client = MozcClient::connect(socket_path)?;
        Ok(Self::from_client(client))
    }

    /// Like `connect` but using the non-spawning quick path (for reconnection).
    pub fn connect_quick(socket_path: Option<&std::path::Path>) -> Result<Self, MozcError> {
        let client = MozcClient::connect_quick(socket_path)?;
        Ok(Self::from_client(client))
    }

    fn from_client(client: MozcClient) -> Self {
        let session_id = client.session_id();
        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (resp_tx, resp_rx) = mpsc::channel::<BatchResult>();

        let handle = thread::spawn(move || {
            worker_loop(client, &req_rx, &resp_tx);
        });

        Self { req_tx, resp_rx, handle: Some(handle), dead: false, session_id }
    }

    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Run a batch of ops on the worker thread, waiting at most `WORKER_TIMEOUT`.
    ///
    /// On timeout (or a dead thread) the worker becomes sticky-dead and every
    /// later call returns `WorkerError::Dead` immediately, so the caller can
    /// discard it and reconnect without ever blocking the UI again.
    pub fn batch(&mut self, ops: Vec<Op>) -> Result<BatchResult, WorkerError> {
        if self.dead {
            return Err(WorkerError::Dead);
        }
        if self.req_tx.send(Request::Batch(ops)).is_err() {
            self.dead = true;
            return Err(WorkerError::Dead);
        }
        match self.resp_rx.recv_timeout(WORKER_TIMEOUT) {
            Ok(results) => Ok(results),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                self.dead = true;
                Err(WorkerError::Dead)
            }
        }
    }
}

impl Drop for MozcWorker {
    fn drop(&mut self) {
        // Ask the worker to stop; it drops the MozcClient (DELETE_SESSION) and exits.
        let _ = self.req_tx.send(Request::Shutdown);
        let Some(handle) = self.handle.take() else { return };
        // A dead worker may be stuck in a slow/hung IPC (that is why it timed out).
        // Joining it here would re-block the UI thread, so detach instead: the
        // orphaned thread exits on its own once the in-flight op finishes (its
        // response send fails because the receiver is gone) and the client drops.
        if !self.dead {
            let _ = handle.join();
        }
    }
}

fn worker_loop(client: MozcClient, req_rx: &Receiver<Request>, resp_tx: &Sender<BatchResult>) {
    while let Ok(req) = req_rx.recv() {
        match req {
            Request::Batch(ops) => {
                let results: BatchResult = ops.iter().map(|op| apply_op(&client, op)).collect();
                // If the receiver timed out and went away, stop; the worker is being
                // discarded by the engine.
                if resp_tx.send(results).is_err() {
                    break;
                }
            }
            Request::Shutdown => break,
        }
    }
    // `client` drops here → DELETE_SESSION sent on the worker thread.
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A scriptable in-process session: each call pops the next queued result.
    struct MockSession {
        calls: RefCell<Vec<String>>,
        replies: RefCell<Vec<Result<MozcOutput, MozcError>>>,
    }

    impl MockSession {
        fn new(replies: Vec<Result<MozcOutput, MozcError>>) -> Self {
            Self { calls: RefCell::new(Vec::new()), replies: RefCell::new(replies) }
        }
        fn next(&self, label: &str) -> Result<MozcOutput, MozcError> {
            self.calls.borrow_mut().push(label.to_string());
            self.replies.borrow_mut().remove(0)
        }
    }

    fn out(preedit: &str) -> MozcOutput {
        MozcOutput { preedit: preedit.to_string(), consumed: true, ..Default::default() }
    }

    impl MozcSession for MockSession {
        fn send_kana(&self, kana: &str) -> Result<MozcOutput, MozcError> { self.next(&format!("kana:{kana}")) }
        fn send_backspace(&self) -> Result<MozcOutput, MozcError> { self.next("bs") }
        fn submit(&self) -> Result<MozcOutput, MozcError> { self.next("submit") }
        fn revert(&self) -> Result<MozcOutput, MozcError> { self.next("revert") }
        fn send_function_key(&self, name: &str) -> Result<MozcOutput, MozcError> { self.next(&format!("fn:{name}")) }
        fn send_modified_key(&self, key: &str, mods: u8) -> Result<MozcOutput, MozcError> { self.next(&format!("mod:{key}:{mods}")) }
    }

    #[test]
    fn apply_op_maps_each_variant() {
        let session = MockSession::new(vec![
            Ok(out("き")), Ok(out("")), Ok(out("")),
        ]);
        assert_eq!(apply_op(&session, &Op::SendKana("き".into())).unwrap().preedit, "き");
        apply_op(&session, &Op::Backspace).unwrap();
        apply_op(&session, &Op::Submit).unwrap();
        assert_eq!(*session.calls.borrow(), vec!["kana:き", "bs", "submit"]);
    }

    #[test]
    fn batch_runs_ops_in_order_via_apply_op() {
        // Simulate a chord rewrite: BackSpace, SendKana — applied left to right.
        let session = MockSession::new(vec![Ok(out("")), Ok(out("を"))]);
        let ops = [Op::Backspace, Op::SendKana("を".into())];
        let results: BatchResult = ops.iter().map(|op| apply_op(&session, op)).collect();
        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok());
        assert_eq!(results[1].as_ref().unwrap().preedit, "を");
        assert_eq!(*session.calls.borrow(), vec!["bs", "kana:を"]);
    }

    #[test]
    fn batch_preserves_per_op_errors() {
        let session = MockSession::new(vec![
            Ok(out("あ")),
            Err(MozcError::NotConnected),
        ]);
        let ops = [Op::SendKana("あ".into()), Op::Submit];
        let results: BatchResult = ops.iter().map(|op| apply_op(&session, op)).collect();
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(MozcError::NotConnected)));
    }
}
