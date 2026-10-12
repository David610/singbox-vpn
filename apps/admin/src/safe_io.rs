//! Output that can never abort a transaction.
//!
//! `println!`/`eprintln!` panic when the stream is a pipe whose reader has gone
//! away (`vpn-admin user create … 2>&1 | head -1`, a supervisor that closed its
//! end, `ssh` dropping mid-command). A panic is not a `Result`: it bypasses the
//! rollback code that every mutating command wraps around "install the new
//! configuration → reload sing-box", so a broken stderr between those two
//! steps would leave the desired configuration, the rendered file and the
//! running service disagreeing.
//!
//! `main.rs` therefore shadows the four std print macros with the ones below
//! (textual macro scope: every module declared after `#[macro_use] mod safe_io;`
//! gets them). They write best-effort and never fail the caller; a write error
//! is remembered, and only a closed pipe on a **read-only** command maps to the
//! conventional SIGPIPE exit status 141 (see `main`). A mutating command that
//! finished its transaction reports its real result — the work is done and
//! reporting "interrupted" would invite a harmful retry.
//!
//! Output is never a place to hide a failure: errors are still returned as
//! `Err` and printed by `main`, and non-print panics keep the default hook.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static OUTPUT_CLOSED: AtomicBool = AtomicBool::new(false);
static MUTATING: AtomicBool = AtomicBool::new(false);

/// Called once the command is known: a mutating command must complete (and
/// report) regardless of what happens to its output streams.
pub fn set_mutating(mutating: bool) {
    MUTATING.store(mutating, Ordering::SeqCst);
}

/// True when a write hit a closed pipe and the command was read-only.
pub fn reader_went_away_on_read_only_command() -> bool {
    OUTPUT_CLOSED.load(Ordering::SeqCst) && !MUTATING.load(Ordering::SeqCst)
}

fn note(result: std::io::Result<()>) {
    if let Err(error) = result {
        if error.kind() == std::io::ErrorKind::BrokenPipe {
            OUTPUT_CLOSED.store(true, Ordering::SeqCst);
        }
    }
}

pub fn stdout(args: std::fmt::Arguments<'_>) {
    note(std::io::stdout().lock().write_fmt(args));
}

pub fn stderr(args: std::fmt::Arguments<'_>) {
    note(std::io::stderr().lock().write_fmt(args));
}

macro_rules! println {
    () => { $crate::safe_io::stdout(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::safe_io::stdout(format_args!("{}\n", format_args!($($arg)*))) };
}
macro_rules! print {
    ($($arg:tt)*) => { $crate::safe_io::stdout(format_args!($($arg)*)) };
}
macro_rules! eprintln {
    () => { $crate::safe_io::stderr(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::safe_io::stderr(format_args!("{}\n", format_args!($($arg)*))) };
}
macro_rules! eprint {
    ($($arg:tt)*) => { $crate::safe_io::stderr(format_args!($($arg)*)) };
}
