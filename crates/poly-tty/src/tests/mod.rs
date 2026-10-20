//! Host tests: the line discipline on its own (`ldisc.rs`), terminals
//! with job control against a fake host (`tty.rs`), and pseudo-terminals
//! through a real VFS with threads (`pty.rs`).

mod fake;
mod ldisc;
mod pty;
mod tty;
