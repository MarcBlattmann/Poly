//! The guest's screen: screenshots through QEMU's machine protocol (QMP),
//! the PPM images QEMU writes, and the boot screen the kernel should show.
//!
//! QMP (QEMU's `docs/interop/qmp-spec.rst`) is JSON, one object per line,
//! over a socket that QEMU opens with `-qmp unix:<path>,server=on,wait=off`.
//! QEMU greets, the client sends `qmp_capabilities`, then commands; each
//! command gets a `return` (or `error`) line, and events may come in
//! between. `screendump` writes what the display shows to a file, also
//! with `-display none`.

use std::path::Path;
use std::time::Duration;

/// An RGB image, 8 bits per channel, rows top to bottom.
#[derive(Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Image {
    /// A binary PPM (`P6`, maximum value 255), as QEMU's `screendump`
    /// writes it.
    pub fn parse_ppm(data: &[u8]) -> Result<Image, String> {
        // Four whitespace-separated header fields, then one whitespace
        // byte, then the pixels.
        let mut fields = Vec::new();
        let mut i = 0;
        while fields.len() < 4 {
            while data.get(i).is_some_and(u8::is_ascii_whitespace) {
                i += 1;
            }
            let start = i;
            while data.get(i).is_some_and(|b| !b.is_ascii_whitespace()) {
                i += 1;
            }
            if start == i {
                return Err("PPM header cut short".into());
            }
            fields.push(String::from_utf8_lossy(&data[start..i]).into_owned());
        }
        let num = |s: &str| s.parse::<u32>().map_err(|_| format!("bad PPM header field {s:?}"));
        if fields[0] != "P6" || num(&fields[3])? != 255 {
            return Err(format!("not an 8-bit binary PPM (header {fields:?})"));
        }
        let (width, height) = (num(&fields[1])?, num(&fields[2])?);
        let len = width as usize * height as usize * 3;
        let rgb = data.get(i + 1..i + 1 + len).ok_or("PPM pixel data cut short")?.to_vec();
        Ok(Image { width, height, rgb })
    }

    pub fn read_ppm(path: &Path) -> Result<Image, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Image::parse_ppm(&data).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn write_ppm(&self, path: &Path) -> Result<(), String> {
        let mut data = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        data.extend_from_slice(&self.rgb);
        std::fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let i = (y as usize * self.width as usize + x as usize) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
}

/// The boot screen of kernel `version` on a `width` × `height` display,
/// drawn by the same code as in the kernel (`poly_gfx::boot_screen`).
pub fn boot_screen(width: u32, height: u32, version: &str) -> Image {
    use poly_gfx::{Canvas, PixelFormat};
    let stride = width as usize * 4;
    let mut buf = vec![0u8; stride * height as usize];
    let mut canvas = Canvas::new(&mut buf, width, height, stride, PixelFormat::Xrgb8888).expect("sized above");
    poly_gfx::boot_screen::draw(&mut canvas, version);
    // XRGB8888 is stored B, G, R, unused.
    let rgb = buf.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
    Image { width, height, rgb }
}

/// Ok if the images are the same; otherwise how they differ.
pub fn compare(got: &Image, want: &Image) -> Result<(), String> {
    if (got.width, got.height) != (want.width, want.height) {
        return Err(format!("the screen is {}x{}, expected {}x{}", got.width, got.height, want.width, want.height));
    }
    let mut diff = 0usize;
    let (mut lo, mut hi) = ((u32::MAX, u32::MAX), (0, 0));
    for y in 0..got.height {
        for x in 0..got.width {
            if got.pixel(x, y) != want.pixel(x, y) {
                diff += 1;
                (lo, hi) = ((lo.0.min(x), lo.1.min(y)), (hi.0.max(x), hi.1.max(y)));
            }
        }
    }
    if diff == 0 {
        return Ok(());
    }
    let (x, y) = lo;
    Err(format!(
        "{diff} pixels differ, within ({}, {})-({}, {}); at ({x}, {y}) the screen has {:02x?}, expected {:02x?}",
        lo.0,
        lo.1,
        hi.0,
        hi.1,
        got.pixel(x, y),
        want.pixel(x, y)
    ))
}

/// A QMP connection.
#[cfg(unix)]
pub struct Qmp {
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
    writer: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
impl Qmp {
    /// Connect to the socket at `path` (waiting up to `timeout` for QEMU to
    /// create it) and leave capabilities negotiation mode.
    pub fn connect(path: &Path, timeout: Duration) -> Result<Qmp, String> {
        use std::io::BufRead;
        use std::os::unix::net::UnixStream;
        let deadline = std::time::Instant::now() + timeout;
        let stream = loop {
            match UnixStream::connect(path) {
                Ok(s) => break s,
                Err(e) if std::time::Instant::now() > deadline => {
                    return Err(format!("QMP socket {}: {e}", path.display()));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(20))).map_err(|e| e.to_string())?;
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        let mut qmp = Qmp { reader: std::io::BufReader::new(stream), writer };
        let mut greeting = String::new();
        qmp.reader.read_line(&mut greeting).map_err(|e| format!("QMP greeting: {e}"))?;
        if !greeting.contains("\"QMP\"") {
            return Err(format!("not a QMP greeting: {greeting:?}"));
        }
        qmp.execute("qmp_capabilities", "{}")?;
        Ok(qmp)
    }

    /// Run `command` with `arguments` (a JSON object) and return its
    /// `return` line.
    pub fn execute(&mut self, command: &str, arguments: &str) -> Result<String, String> {
        use std::io::{BufRead, Write};
        writeln!(self.writer, "{{\"execute\": \"{command}\", \"arguments\": {arguments}}}")
            .map_err(|e| format!("QMP {command}: {e}"))?;
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) => return Err(format!("QMP {command}: QEMU closed the connection")),
                Ok(_) if line.contains("\"return\"") => return Ok(line),
                Ok(_) if line.contains("\"error\"") => return Err(format!("QMP {command}: {}", line.trim())),
                // An event; keep waiting for the answer.
                Ok(_) => {}
                Err(e) => return Err(format!("QMP {command}: {e}")),
            }
        }
    }

    /// Write what the display shows to `file`: PNG if its name ends in
    /// `.png`, else PPM.
    pub fn screendump(&mut self, file: &Path) -> Result<(), String> {
        let name = file.to_str().ok_or("screenshot path is not UTF-8")?;
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        let format = if name.ends_with(".png") { ", \"format\": \"png\"" } else { "" };
        self.execute("screendump", &format!("{{\"filename\": \"{escaped}\"{format}}}")).map(drop)
    }

    /// Ask QEMU to exit.
    pub fn quit(&mut self) {
        let _ = self.execute("quit", "{}");
    }
}

/// Connect to QEMU's QMP socket at `socket`, save a screenshot to `file`
/// and make QEMU quit.
#[cfg(unix)]
pub fn screenshot(socket: &Path, file: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(file);
    let mut qmp = Qmp::connect(socket, Duration::from_secs(10))?;
    let result = qmp.screendump(file);
    qmp.quit();
    result?;
    if !file.exists() {
        return Err(format!("QEMU reported a screenshot but {} is missing", file.display()));
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn screenshot(_socket: &Path, _file: &Path) -> Result<(), String> {
    Err("screenshots go through a Unix socket to QEMU, which this host does not have".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ppm() {
        let mut data = b"P6\n2 1\n255\n".to_vec();
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let img = Image::parse_ppm(&data).unwrap();
        assert_eq!((img.width, img.height, img.pixel(1, 0)), (2, 1, [4, 5, 6]));
        assert!(Image::parse_ppm(&data[..data.len() - 1]).unwrap_err().contains("cut short"));
        assert!(Image::parse_ppm(b"P5\n1 1\n255\n\0").unwrap_err().contains("not an 8-bit binary PPM"));
        assert!(Image::parse_ppm(b"P6\n1 1\n65535\n\0\0\0\0\0\0").is_err());
        assert!(Image::parse_ppm(b"P6 1").unwrap_err().contains("cut short"));
        // Whitespace in the header is free-form.
        assert!(Image::parse_ppm(b"P6  1\t1 \n255\n\x01\x02\x03").is_ok());
    }

    #[test]
    fn compare_reports_the_differences() {
        let a = boot_screen(320, 200, "1.2.3");
        assert_eq!(compare(&a, &boot_screen(320, 200, "1.2.3")), Ok(()));
        let other = boot_screen(320, 200, "1.2.4");
        let e = compare(&other, &a).unwrap_err();
        assert!(e.contains("pixels differ"), "{e}");
        assert!(compare(&a, &boot_screen(200, 320, "1.2.3")).unwrap_err().contains("is 320x200, expected 200x320"));
        // The corners are the background colour.
        let bg = poly_gfx::boot_screen::BACKGROUND;
        assert_eq!(a.pixel(0, 0), [bg.r(), bg.g(), bg.b()]);
        let dir = std::env::temp_dir().join(format!("poly-xtask-test-{}.ppm", std::process::id()));
        a.write_ppm(&dir).unwrap();
        assert_eq!(Image::read_ppm(&dir).unwrap(), a);
        let _ = std::fs::remove_file(dir);
    }
}
