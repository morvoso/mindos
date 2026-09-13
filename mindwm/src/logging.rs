// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! Logging that never waits for the journal.
//!
//! In a session the compositor's standard output is a journald stream socket
//! (`systemd-cat`). When journald falls behind -- a burst of lines, a busy
//! disk, journald restarting -- a plain write to that socket blocks, and a
//! write made from the event loop stops every display and the pointer until
//! it returns. So lines are handed to a thread that does the writing. When
//! that thread cannot keep up, new lines are counted and dropped rather than
//! waited for, and the count is written once it can.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing_subscriber::fmt::MakeWriter;

/// Lines waiting for the writer before new ones are dropped: a few seconds
/// of the worst bursts the compositor produces.
const QUEUE: usize = 8192;
/// How long the end of the process waits for queued lines to be written.
const FLUSH_AT_EXIT: Duration = Duration::from_secs(1);

/// Set up the global subscriber, filtered by `RUST_LOG` when that is set and
/// by `default_filter` otherwise. Keep the returned guard alive for the life
/// of the process: dropping it writes out what is still queued.
pub fn init(default_filter: &str) -> Flush {
    let writer = Writer::spawn();
    let flush = Flush(writer.shared.clone());
    let ansi = io::stdout().is_terminal();
    let builder = tracing_subscriber::fmt().compact().with_ansi(ansi).with_writer(writer);
    match tracing_subscriber::EnvFilter::try_from_default_env() {
        Ok(filter) => builder.with_env_filter(filter).init(),
        Err(_) => builder.with_env_filter(default_filter).init(),
    }
    flush
}

struct Shared {
    /// Lines handed over but not yet written.
    queued: AtomicUsize,
    /// Lines dropped since the writer last said so.
    dropped: AtomicU64,
}

/// The `MakeWriter` the subscriber formats into: one `Line` per event.
pub struct Writer {
    /// `None` when the writing thread could not be started; lines are then
    /// written in place, the way they always were.
    tx: Option<SyncSender<Vec<u8>>>,
    shared: Arc<Shared>,
}

impl Writer {
    fn spawn() -> Writer {
        let shared = Arc::new(Shared {
            queued: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        });
        let (tx, rx) = sync_channel(QUEUE);
        let thread_shared = shared.clone();
        let tx = std::thread::Builder::new()
            .name("mindwm-log".into())
            .spawn(move || write_lines(rx, thread_shared))
            .map(|_| tx)
            .map_err(|err| eprintln!("mindwm: no log thread ({err}); logging straight to stdout"))
            .ok();
        Writer { tx, shared }
    }

    fn send(&self, line: Vec<u8>) {
        let Some(tx) = self.tx.as_ref() else {
            let _ = io::stdout().lock().write_all(&line);
            return;
        };
        self.shared.queued.fetch_add(1, Ordering::SeqCst);
        if tx.try_send(line).is_err() {
            self.shared.queued.fetch_sub(1, Ordering::SeqCst);
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl<'a> MakeWriter<'a> for Writer {
    type Writer = Line<'a>;

    fn make_writer(&'a self) -> Line<'a> {
        Line {
            buf: Vec::new(),
            writer: self,
        }
    }
}

/// One formatted event, sent as a whole when the subscriber lets go of it.
pub struct Line<'a> {
    buf: Vec<u8>,
    writer: &'a Writer,
}

impl Write for Line<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Line<'_> {
    fn drop(&mut self) {
        if !self.buf.is_empty() {
            self.writer.send(std::mem::take(&mut self.buf));
        }
    }
}

fn write_lines(rx: Receiver<Vec<u8>>, shared: Arc<Shared>) {
    let mut stdout = io::stdout();
    let mut batch = Vec::with_capacity(64 * 1024);
    while let Ok(line) = rx.recv() {
        // Take everything already waiting, so a burst costs one write.
        let mut lines = 1;
        batch.extend_from_slice(&line);
        while batch.len() < 256 * 1024 {
            match rx.try_recv() {
                Ok(line) => {
                    batch.extend_from_slice(&line);
                    lines += 1;
                }
                Err(_) => break,
            }
        }
        let dropped = shared.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            let _ = writeln!(
                batch,
                "mindwm: {dropped} log lines were dropped because the journal was not taking them fast enough"
            );
        }
        let _ = stdout.write_all(&batch);
        let _ = stdout.flush();
        batch.clear();
        shared.queued.fetch_sub(lines, Ordering::SeqCst);
    }
}

/// Writes out what is still queued when dropped, for up to a second.
pub struct Flush(Arc<Shared>);

impl Drop for Flush {
    fn drop(&mut self) {
        let until = Instant::now() + FLUSH_AT_EXIT;
        while self.0.queued.load(Ordering::SeqCst) > 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
