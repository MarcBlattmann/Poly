//! The terminal, with plain ANSI and termios (no crate): the alternate
//! screen, keys without echo or line buffering, the window size, and
//! everything put back on exit, on `q`, Ctrl-C, SIGINT, SIGTERM, SIGHUP
//! and on a panic (the guard's `Drop`).

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static QUIT: AtomicBool = AtomicBool::new(false);
static RESIZED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_quit(_: libc::c_int) {
    QUIT.store(true, Ordering::Relaxed);
}

extern "C" fn on_resize(_: libc::c_int) {
    RESIZED.store(true, Ordering::Relaxed);
}

fn handle(signal: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    // SAFETY: sigaction is plain data; all zeroes is a valid (empty) value.
    let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
    sa.sa_sigaction = handler as libc::sighandler_t;
    // SAFETY: `sa.sa_mask` is a valid sigset_t to initialise.
    unsafe { libc::sigemptyset(&mut sa.sa_mask) };
    // No SA_RESTART: the signal ends the wait in poll(2) at once. The
    // handlers only store to atomics (async-signal-safe).
    sa.sa_flags = 0;
    // SAFETY: `sa` is initialised and the handler is an `extern "C" fn`
    // that lives for the whole program.
    unsafe { libc::sigaction(signal, &sa, std::ptr::null_mut()) };
}

pub fn quit_requested() -> bool {
    QUIT.load(Ordering::Relaxed)
}

pub fn take_resized() -> bool {
    RESIZED.swap(false, Ordering::Relaxed)
}

pub fn is_tty(fd: libc::c_int) -> bool {
    // SAFETY: isatty only looks at the descriptor number.
    unsafe { libc::isatty(fd) == 1 }
}

/// (columns, rows) of the terminal on stdout.
pub fn size() -> Option<(usize, usize)> {
    // SAFETY: winsize is plain integers; all zeroes is a valid value.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ writes a `winsize` to the pointer, which is valid.
    let r = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    (r == 0 && ws.ws_col > 0 && ws.ws_row > 0).then(|| (usize::from(ws.ws_col), usize::from(ws.ws_row)))
}

/// The terminal in dashboard mode while this lives.
#[derive(Debug)]
pub struct Screen {
    saved: Option<libc::termios>,
    stdin_done: bool,
}

impl Screen {
    pub fn enter() -> Screen {
        handle(libc::SIGINT, on_quit);
        handle(libc::SIGTERM, on_quit);
        handle(libc::SIGHUP, on_quit);
        handle(libc::SIGWINCH, on_resize);
        let mut saved = None;
        if is_tty(libc::STDIN_FILENO) {
            // SAFETY: termios is plain data; all zeroes is a valid value.
            let mut t: libc::termios = unsafe { std::mem::zeroed() };
            // SAFETY: `t` is a valid termios for tcgetattr to fill.
            if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut t) } == 0 {
                saved = Some(t);
                // Keys one by one, no echo, Ctrl-C as a key (0x03); output
                // processing stays on, so "\n" still starts a new line.
                t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 0;
                // SAFETY: `t` is the terminal's own settings, changed above.
                unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t) };
            }
        }
        // Alternate screen, cursor hidden.
        print_flush("\x1b[?1049h\x1b[?25l\x1b[H\x1b[2J");
        Screen { saved, stdin_done: false }
    }

    /// Draw `lines` (each already cut to the width) over the last frame.
    pub fn draw(&self, lines: &[String]) {
        let mut frame = String::from("\x1b[H");
        for (i, l) in lines.iter().enumerate() {
            if i > 0 {
                frame.push('\n');
            }
            frame.push_str(l);
            frame.push_str("\x1b[0m\x1b[K");
        }
        frame.push_str("\x1b[J");
        print_flush(&frame);
    }

    /// Wait up to `ms` for a key; `Some(byte)` for a key, `None` when the
    /// time is up or a signal came.
    pub fn key(&mut self, ms: u64) -> Option<u8> {
        let timeout = libc::c_int::try_from(ms).unwrap_or(libc::c_int::MAX);
        let mut fds = [libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 }];
        let n = u32::from(!self.stdin_done) as libc::nfds_t;
        // SAFETY: `fds` holds `n` (0 or 1) valid pollfd entries; with none
        // poll(2) just sleeps until the timeout or a signal.
        let r = unsafe { libc::poll(fds.as_mut_ptr(), n, timeout) };
        if r <= 0 || fds[0].revents == 0 {
            return None;
        }
        let mut b = [0u8; 16];
        // SAFETY: reads at most `b.len()` bytes into `b`.
        let got = unsafe { libc::read(libc::STDIN_FILENO, b.as_mut_ptr().cast(), b.len()) };
        if got <= 0 {
            // End of input (stdin is not a terminal): only signals now.
            self.stdin_done = true;
            return None;
        }
        // Escape sequences (arrow keys) are ignored as a whole.
        if b[0] == 0x1b { None } else { Some(b[0]) }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        print_flush("\x1b[0m\x1b[?25h\x1b[?1049l");
        if let Some(t) = &self.saved {
            // SAFETY: restores the settings tcgetattr returned.
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t) };
        }
    }
}

fn print_flush(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}
