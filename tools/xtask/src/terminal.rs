//! Boot test for the screen terminal (kernel/src/terminal.rs): keys typed
//! on the guest's PS/2 keyboard through QEMU's monitor reach the shell on
//! the screen, and screenshots through QMP show the command's output and
//! the status bar.
//!
//! The screen is compared with what `poly-gfx` and `poly-term` draw on the
//! host for the same layout: the output row cell by cell (the text drawn
//! by a `TextGrid` in the terminal's palette) and the status bar's fixed
//! start (" Poly <version>"), whose other pixels must all be the bar's two
//! colours. Screenshots are taken every half second until the screen
//! matches or the boot times out, so a slow (loaded) host only makes the
//! test slower.

use crate::screen::{self, Image};
use crate::symbols::Symbols;
use crate::{BOOT_TIMEOUT, DEFAULT_MEMORY, Devices, Result, TEST_DISPLAY_FLAG, ktest, qemu, send_keys};
use poly_gfx::{Canvas, Cell, Palette, PixelFormat, TextGrid};
use poly_term::Screen;
use poly_term::status::{self, StatusInfo};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// The kernel prints this once the screen terminal and its shell run (and
/// the serial shell is about to start).
const STARTED: &str = "boot: done; starting the shell on COM1";
/// What the test types (QEMU `sendkey` names): "ech", Tab (completes
/// "echo "), "Polx", Backspace, "y-Term", Enter.
const KEYS: [&str; 16] =
    ["e", "c", "h", "tab", "shift-p", "o", "l", "x", "backspace", "y", "minus", "shift-t", "e", "r", "m", "ret"];
/// What the command prints.
const OUTPUT: &str = "Poly-Term";
/// The display mode of the test boot.
const MODE: (u32, u32) = (1024, 768);

/// Boot into the screen terminal, type a command on the keyboard and check
/// the screen.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>, version: &str) -> Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: screenshots and the QEMU monitor need a Unix host");
        return Ok(());
    }
    let (w, h) = MODE;
    let target = crate::root().join("target");
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let file = target.join("poly-terminal-test.ppm");
    let id = std::process::id();
    let qmp = std::env::temp_dir().join(format!("poly-term-qmp-{id}.sock"));
    let monitor = std::env::temp_dir().join(format!("poly-term-monitor-{id}.sock"));
    for s in [&qmp, &monitor] {
        let _ = std::fs::remove_file(s);
    }
    let append = format!("poly.test=terminal:: {TEST_DISPLAY_FLAG} poly.display={w}x{h}");
    let mut child = qemu(kernel, DEFAULT_MEMORY, &append, Devices::None, "stdio")
        .args(["-display", "none"])
        .arg("-qmp")
        .arg(format!("unix:{},server=on,wait=off", qmp.display()))
        .arg("-monitor")
        .arg(format!("unix:{},server=on,wait=off", monitor.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("qemu-system-x86_64: {e} (is QEMU installed?)"))?;
    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = mpsc::channel();
    let syms = Arc::clone(syms);
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(std::result::Result::ok) {
            let line = syms.symbolize_line(&line);
            println!("    | {line}");
            let _ = tx.send(line);
        }
    });

    let result = drive(&rx, &qmp, &monitor, &file, version);
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    for s in [&qmp, &monitor] {
        let _ = std::fs::remove_file(s);
    }
    result.map_err(|e| format!("{e} (last screenshot: {})", file.display()))
}

/// Wait for the terminal, type, and compare screenshots until they match.
fn drive(rx: &mpsc::Receiver<String>, qmp: &Path, monitor: &Path, file: &Path, version: &str) -> Result {
    let deadline = Instant::now() + BOOT_TIMEOUT;
    let mut log = String::new();
    loop {
        let line = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| format!("the kernel never printed \"{STARTED}\""))?;
        log.push_str(&line);
        log.push('\n');
        if line.contains("POLY-SELFTEST: FAIL") || line.contains("KERNEL PANIC") {
            ktest::parse(&log).check()?;
            return Err(format!("the kernel printed \"{}\"", line.trim()));
        }
        if line.contains(STARTED) {
            break;
        }
    }
    ktest::parse(&log).check()?;
    if !log.lines().any(|l| l.starts_with("terminal: ") && l.contains(" cells in Spleen 8x16 on the display")) {
        return Err("the kernel did not report the screen terminal".into());
    }
    let monitor = monitor.to_path_buf();
    let typist = thread::spawn(move || send_keys(&monitor, &KEYS));
    let mut q = screen::Qmp::connect(qmp, Duration::from_secs(10))?;
    let want = Expected::new(MODE.0, MODE.1, version);
    let mut last = String::from("no screenshot taken");
    let outcome = loop {
        if Instant::now() > deadline {
            break Err(format!("the screen never showed the command's output and the status bar: {last}"));
        }
        thread::sleep(Duration::from_millis(500));
        let _ = rx.try_iter().count();
        if let Err(e) = q.screendump(file) {
            last = e;
            continue;
        }
        match Image::read_ppm(file).and_then(|img| want.check(&img)) {
            Ok(row) => {
                eprintln!("==> the screen shows \"{OUTPUT}\" on row {row} and the status bar");
                break Ok(());
            }
            Err(e) => last = e,
        }
    };
    q.quit();
    typist.join().map_err(|_| "the key sender panicked")??;
    outcome
}

/// What the screen should show, drawn on the host.
struct Expected {
    layout: Screen,
    /// The status bar as drawn for the version (the rest of the line
    /// changes: uptime, memory).
    status: Image,
    /// Columns of the status bar that never change.
    title_cols: usize,
    /// [`OUTPUT`] in the terminal's default colours, then a blank cell.
    output: Image,
}

impl Expected {
    fn new(width: u32, height: u32, version: &str) -> Expected {
        let layout = Screen::new(width, height).expect("a terminal fits the test mode");
        let font = layout.font;
        let (cw, ch) = (font.width(), font.height());
        let status = draw(width, ch, |c| {
            let info = StatusInfo {
                version: "",
                uptime_ns: 0,
                free_bytes: 0,
                total_bytes: 0,
                layout: "us",
                cpus_online: 1,
                cpus: 1,
                scrolled: None,
            };
            let line = status::format(&StatusInfo { version: leak(version), ..info }, layout.cols);
            status::draw(c, layout.status, font, line.as_str());
        });
        let cols = OUTPUT.chars().count() + 1;
        let output = draw(cols as u32 * cw, ch, |c| {
            let mut cells = vec![Cell::BLANK; cols];
            let mut grid = TextGrid::new(&mut cells, cols, 1).expect("one row");
            grid.set_cursor_visible(false);
            grid.write_str(OUTPUT);
            grid.render(c, 0, 0, font, &Palette::DEFAULT);
        });
        Expected { layout, status, title_cols: status::title_cols(version), output }
    }

    /// Ok with the grid row that shows the output, or what is wrong.
    fn check(&self, img: &Image) -> std::result::Result<usize, String> {
        let s = &self.layout;
        if (img.width, img.height) != (s.width, s.height) {
            return Err(format!("the screen is {}x{}, expected {}x{}", img.width, img.height, s.width, s.height));
        }
        // The status bar: its fixed start as drawn on the host, and only
        // the bar's colours across the whole row.
        let cw = s.font.width();
        let title_px = self.title_cols as u32 * cw;
        let bar = [status::BACKGROUND, status::FOREGROUND].map(|c| [c.r(), c.g(), c.b()]);
        for y in 0..s.status.height {
            for x in 0..s.width {
                let p = img.pixel(x, y);
                if x < title_px && p != self.status.pixel(x, y) {
                    return Err(format!("the status bar does not start with \" Poly <version>\" (pixel {x}, {y})"));
                }
                if !bar.contains(&p) {
                    return Err(format!("the status bar has a pixel of another colour at ({x}, {y}): {p:02x?}"));
                }
            }
        }
        // Text on the right of the bar too (the scrollback hint).
        let right = (s.width / 2..s.width).any(|x| (0..s.status.height).any(|y| img.pixel(x, y) == bar[1]));
        if !right {
            return Err("the status bar has no text on its right half".into());
        }
        // A grid row that starts with the command's output.
        let ch = s.font.height();
        let found = (0..s.rows).find(|&row| {
            let top = s.grid.y as u32 + row as u32 * ch;
            (0..ch).all(|y| (0..self.output.width).all(|x| img.pixel(x, top + y) == self.output.pixel(x, y)))
        });
        found.ok_or_else(|| format!("no row of the screen starts with \"{OUTPUT}\""))
    }
}

/// A `width` × `height` image drawn by `f` (on an XRGB8888 canvas).
fn draw(width: u32, height: u32, f: impl FnOnce(&mut Canvas<'_>)) -> Image {
    let stride = width as usize * 4;
    let mut buf = vec![0u8; stride * height as usize];
    let mut canvas = Canvas::new(&mut buf, width, height, stride, PixelFormat::Xrgb8888).expect("sized above");
    f(&mut canvas);
    // XRGB8888 is stored B, G, R, unused.
    let rgb = buf.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
    Image { width, height, rgb }
}

/// `StatusInfo` takes `&'static str`s, as the kernel's are.
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checker accepts what the terminal draws and rejects a screen
    /// without the output.
    #[test]
    fn expected_matches_a_host_rendered_terminal() {
        let (w, h) = MODE;
        let want = Expected::new(w, h, "0.0.1");
        let layout = Screen::new(w, h).unwrap();
        let mut cells = vec![Cell::BLANK; layout.cells()];
        let mut t = poly_term::Terminal::new(layout, &mut cells).unwrap();
        t.set_status(StatusInfo {
            version: "0.0.1",
            uptime_ns: 12_345_000_000_000,
            free_bytes: 200 << 20,
            total_bytes: 256 << 20,
            layout: "us",
            cpus_online: 1,
            cpus: 2,
            scrolled: None,
        });
        t.write_log(b"boot: done\n");
        t.write_shell("\x1b[1;32mpoly>\x1b[0m echo Poly-Term\n");
        let blank = draw(w, h, |c| {
            t.render(c);
        });
        assert!(want.check(&blank).unwrap_err().contains("no row"), "no output yet");
        t.write_shell("Poly-Term\n\x1b[1;32mpoly>\x1b[0m ");
        let shown = draw(w, h, |c| {
            t.invalidate();
            t.render(c);
        });
        assert_eq!(want.check(&shown), Ok(2));
        let other = Expected::new(w, h, "9.9.9");
        assert!(other.check(&shown).unwrap_err().contains("does not start with"));
    }
}
