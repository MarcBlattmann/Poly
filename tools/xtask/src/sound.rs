//! The sound boot test (ADR 0104): `/bin/pcm-tone`
//! (`tests/linux/audio/pcm_tone.c`, linked statically with the host's C
//! library) plays sine tones through `/dev/snd/pcmC0D0p` on QEMU's
//! virtio-sound device, whose `wav` audio backend writes what the guest
//! played to a file on the host (QEMU documentation, "Audio options":
//! `-audiodev wav,id=id,path=path`). The test then reads that file and
//! checks what was heard, not bytes: windows of 100 ms whose frequency
//! (from the zero crossings) and level (RMS) match each tone the program
//! played, on both channels.
//!
//! The program plays 440 Hz at 48 kHz through `SNDRV_PCM_IOCTL_WRITEI_FRAMES`
//! and 1000 Hz at 44.1 kHz through `write(2)`, each for 1.5 s at half of
//! full scale, then runs an underrun and its recovery. The wav backend
//! records at its own fixed settings (44.1 kHz, stereo, S16 by default),
//! so QEMU resamples and converts the other streams; frequencies and
//! levels survive that. One boot is interrupt-driven (MSI-X), one polled
//! (`vectors=0`) with two other formats and rates: 660 Hz in U8 at 22.05
//! kHz and 880 Hz in FLOAT at 48 kHz (through `write(2)`).
//!
//! A first boot runs the kernel's own sound tests (`poly.test=sound::`)
//! with the device attached. Skipped, with a message, when QEMU has no
//! `virtio-sound-pci` or there is no C compiler with a static C library.
//!
//! ADR 0109 adds, each checked in the recording the same way:
//!
//! - **mmap and the mixer controls**: the master volume set to 50 through
//!   the control device, then 523 Hz played through the mapped ring buffer
//!   (status and control pages, `SYNC_PTR`): heard at half the level;
//! - **the audio server**: two processes play 440 Hz and 660 Hz at once,
//!   each at a quarter of full scale: both tones are found in the same
//!   windows at their level (a single-frequency DFT per window, since zero
//!   crossings cannot tell a sum of tones apart);
//! - **capture** from `/dev/snd/pcmC0D0c`: QEMU's `wav` backend only
//!   records what is played and offers no input (QEMU documentation,
//!   "Audio options"), so this boot uses the `none` backend, whose input
//!   is silence delivered in real time; the program checks the timing, the
//!   reads and the overrun, and that what came is silence;
//! - **Intel HDA**: the kernel tests and the two default tones again on
//!   `-device intel-hda -device hda-duplex`, recorded by the `wav`
//!   backend, when QEMU has those devices.

use super::{DEFAULT_MEMORY, Devices, PASS_MARKER, QEMU_SUCCESS, Result, Symbols, boot_with, initramfs, root};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const SOURCE: &str = "tests/linux/audio/pcm_tone.c";
/// The last line `/bin/pcm-tone` writes when every check passed.
const DONE: &str = "pcm-tone: done, 0 failed";
/// The tones' amplitude (`AMPLITUDE` in the program): half of full scale.
const AMPLITUDE: f64 = 16384.0;

/// Build `/bin/pcm-tone`; `Ok(None)` (with the reason printed) where there
/// is no C compiler or static C library.
fn build() -> Result<Option<Vec<u8>>> {
    let out = root().join("target/xtask-programs");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let binary = out.join("pcm-tone");
    let built = Command::new("cc")
        .current_dir(root())
        .args(["-static", "-fno-pie", "-no-pie", "-O2", "-Wall", "-Werror", "-Wl,--build-id=none", "-o"])
        .arg(&binary)
        .arg(SOURCE)
        .arg("-lm")
        .output();
    match built {
        Ok(o) if o.status.success() => {
            Ok(Some(std::fs::read(&binary).map_err(|e| format!("{}: {e}", binary.display()))?))
        }
        Ok(o) => {
            let why = String::from_utf8_lossy(&o.stderr);
            if why.contains("cannot find -lc") || why.contains("cannot find -lm") {
                eprintln!("==> skipped: no static C library for {SOURCE}");
                Ok(None)
            } else {
                Err(format!("building {SOURCE} failed:\n{why}"))
            }
        }
        Err(e) => {
            eprintln!("==> skipped: no C compiler for {SOURCE} ({e})");
            Ok(None)
        }
    }
}

/// Does this QEMU have the virtio sound device?
pub(crate) fn qemu_has_virtio_sound() -> bool {
    Command::new("qemu-system-x86_64")
        .args(["-device", "help"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("\"virtio-sound-pci\""))
}

/// A WAV file's PCM samples: 16-bit, interleaved.
#[derive(Debug)]
pub struct Wav {
    pub rate: u32,
    pub channels: usize,
    pub samples: Vec<i16>,
}

/// Read a RIFF/WAVE file of 16-bit PCM. The data chunk runs to the end of
/// the file if its length is 0 or too long (a recorder that was not told
/// the end).
pub fn parse_wav(b: &[u8]) -> Result<Wav> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let le16 = |at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
    let le32 = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let mut at = 12;
    let mut fmt = None;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = le32(at + 4) as usize;
        let body = at + 8;
        if id == b"fmt " && body + 16 <= b.len() {
            fmt = Some((le16(body), le16(body + 2) as usize, le32(body + 4), le16(body + 14)));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt.ok_or("a data chunk before the fmt chunk")?;
            if format != 1 || bits != 16 || channels == 0 {
                return Err(format!("not 16-bit PCM (format {format}, {bits} bits, {channels} channels)"));
            }
            let end = if len == 0 || body + len > b.len() { b.len() } else { body + len };
            let samples = b[body..end].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
            return Ok(Wav { rate, channels, samples });
        }
        at = body + len + (len & 1);
    }
    Err("no data chunk".into())
}

/// What one window of the recording sounds like.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    /// Seconds from the start of the recording.
    pub start: f64,
    /// Root mean square of each of the first two channels.
    pub rms: [f64; 2],
    /// The frequency from the upward zero crossings of channel 0 (0 if
    /// there are fewer than two).
    pub hz: f64,
}

/// Cut the recording into windows of `secs` and measure each.
pub fn windows(w: &Wav, secs: f64) -> Vec<Window> {
    let frames = w.samples.len() / w.channels;
    let len = ((w.rate as f64 * secs) as usize).max(2);
    let ch = |f: usize, c: usize| w.samples[f * w.channels + c.min(w.channels - 1)] as f64;
    (0..frames / len)
        .map(|i| {
            let range = i * len..(i + 1) * len;
            let rms = [0, 1].map(|c| (range.clone().map(|f| ch(f, c).powi(2)).sum::<f64>() / len as f64).sqrt());
            // Upward crossings, placed between the two samples by linear
            // interpolation; the frequency is crossings per second between
            // the first and the last.
            let mut first = None;
            let mut last = 0.0;
            let mut count = 0usize;
            for f in range.start + 1..range.end {
                let (a, b) = (ch(f - 1, 0), ch(f, 0));
                if a < 0.0 && b >= 0.0 {
                    let t = (f - 1) as f64 + a / (a - b);
                    first.get_or_insert(t);
                    last = t;
                    count += 1;
                }
            }
            let hz = match first {
                Some(f) if count >= 2 && last > f => (count - 1) as f64 * w.rate as f64 / (last - f),
                _ => 0.0,
            };
            Window { start: (range.start as f64) / w.rate as f64, rms, hz }
        })
        .collect()
}

/// Windows that are the tone `hz` at the program's level: frequency within
/// 2 %, both channels within 25 % of the sine's RMS.
#[cfg(test)]
pub fn tone_windows(ws: &[Window], hz: f64) -> usize {
    tone_windows_at(ws, hz, AMPLITUDE)
}

/// [`tone_windows`] for a sine of `amplitude`.
pub fn tone_windows_at(ws: &[Window], hz: f64, amplitude: f64) -> usize {
    let rms = amplitude / std::f64::consts::SQRT_2;
    ws.iter().filter(|w| (w.hz - hz).abs() <= hz * 0.02 && w.rms.iter().all(|&r| (r - rms).abs() <= rms * 0.25)).count()
}

/// The amplitude of the `hz` component of channel `c` in each window of
/// `secs`: one bin of a discrete Fourier transform, `2/N |sum x[n]
/// e^(-2 pi i hz n / rate)|`.
pub fn tone_levels(w: &Wav, secs: f64, hz: f64, c: usize) -> Vec<f64> {
    let frames = w.samples.len() / w.channels;
    let len = ((w.rate as f64 * secs) as usize).max(2);
    let k = 2.0 * std::f64::consts::PI * hz / w.rate as f64;
    (0..frames / len)
        .map(|i| {
            let (mut re, mut im) = (0.0, 0.0);
            for n in 0..len {
                let x = w.samples[(i * len + n) * w.channels + c.min(w.channels - 1)] as f64;
                re += x * (k * n as f64).cos();
                im -= x * (k * n as f64).sin();
            }
            2.0 / len as f64 * (re * re + im * im).sqrt()
        })
        .collect()
}

/// Windows in which every tone of `hzs` is there at `amplitude` (within
/// 25 %) on both channels.
pub fn chord_windows(w: &Wav, hzs: &[f64], amplitude: f64) -> usize {
    let levels: Vec<Vec<f64>> = hzs.iter().flat_map(|&hz| [0, 1].map(|c| tone_levels(w, 0.1, hz, c))).collect();
    let n = levels.iter().map(Vec::len).min().unwrap_or(0);
    (0..n).filter(|&i| levels.iter().all(|l| (l[i] - amplitude).abs() <= amplitude * 0.25)).count()
}

/// The sound device a boot attaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Device {
    /// virtio-sound with MSI-X.
    Virtio,
    /// virtio-sound without MSI-X (`vectors=0`): polled.
    VirtioPolled,
    /// Intel HDA (ICH6) with a duplex codec.
    Hda,
}

impl Device {
    fn name(self) -> &'static str {
        match self {
            Device::Virtio => "virtio-sound, interrupt-driven",
            Device::VirtioPolled => "virtio-sound, polled",
            Device::Hda => "Intel HDA",
        }
    }

    fn qemu_args(self) -> Vec<String> {
        match self {
            Device::Virtio => vec!["-device".into(), "virtio-sound-pci,audiodev=snd0".into()],
            Device::VirtioPolled => vec!["-device".into(), "virtio-sound-pci,audiodev=snd0,vectors=0".into()],
            Device::Hda => {
                vec!["-device".into(), "intel-hda".into(), "-device".into(), "hda-duplex,audiodev=snd0".into()]
            }
        }
    }

    /// What the kernel logs for its card.
    fn card_line(self) -> &'static str {
        match self {
            Device::Hda => "sound: card 0: /dev/snd/controlC0, pcmC0D0p; ",
            _ => "sound: card 0: /dev/snd/controlC0, pcmC0D0p, pcmC0D0c; ",
        }
    }

    fn configured(self) -> &'static str {
        match self {
            Device::Virtio => "MSI-X entry 0 -> vector",
            Device::VirtioPolled => "polled every",
            Device::Hda => "; MSI -> vector",
        }
    }
}

/// What a boot's recording must contain.
enum Heard {
    /// Tones (Hz, amplitude, at least this many 100 ms windows).
    Tones(&'static [(f64, f64, usize)]),
    /// Tones heard together (Hz, amplitude each, windows).
    Chord(&'static [f64], f64, usize),
}

/// One boot: `/bin/pcm-tone` with `args` as init on `device`, recorded by
/// QEMU's `wav` backend (or with the `none` backend and no recording when
/// `heard` is empty); the recording is checked for `heard`.
fn boot_and_listen(
    kernel: &Path,
    syms: &Arc<Symbols>,
    image: &Path,
    device: Device,
    args: &[&str],
    heard: &[Heard],
) -> Result {
    let how = device.name();
    let wav: PathBuf = root().join(format!("target/poly-sound-{}.wav", how.replace([' ', ','], "")));
    let _ = std::fs::remove_file(&wav);
    let backend =
        if heard.is_empty() { "none,id=snd0".to_string() } else { format!("wav,id=snd0,path={}", wav.display()) };
    let mut qemu_args = vec!["-initrd".to_string(), image.display().to_string(), "-audiodev".into(), backend];
    qemu_args.extend(device.qemu_args());
    let mut append = "poly.qemu_exit poly.init=/bin/pcm-tone".to_string();
    for a in args {
        append.push_str(&format!(" poly.init.arg={a}"));
    }
    let b = boot_with(kernel, syms, DEFAULT_MEMORY, &append, Devices::None, &qemu_args, None, |_| {})?;
    if let Some(line) = b.output.lines().find(|l| l.contains("pcm-tone: FAIL")) {
        return Err(format!("/bin/pcm-tone ({how}): {line}"));
    }
    let checks = [
        ("the device was found", b.output.contains(device.card_line())),
        ("as configured", b.output.contains(device.configured())),
        ("every check passed", b.output.contains(DONE)),
        ("init exited 0", b.output.contains("POLY-EXIT: pid=1 code=0")),
        ("the stream closed cleanly", b.output.contains("pcmC0D0p closed:")),
    ];
    for (what, ok) in checks {
        if !ok {
            return Err(format!("sound boot ({how}): {what} failed (QEMU status {:?})", b.status));
        }
    }
    if b.status != Some(QEMU_SUCCESS) {
        return Err(format!("sound boot ({how}): QEMU status {:?}", b.status));
    }
    if heard.is_empty() {
        return Ok(());
    }
    let bytes = std::fs::read(&wav).map_err(|e| format!("{}: {e} (QEMU wrote no recording)", wav.display()))?;
    let rec = parse_wav(&bytes).map_err(|e| format!("{}: {e}", wav.display()))?;
    let ws = windows(&rec, 0.1);
    let what_was_heard = || {
        let heard: Vec<String> = ws
            .iter()
            .filter(|w| w.rms[0] > 1000.0)
            .map(|w| format!("{:.1}s {:.0} Hz rms {:.0}/{:.0}", w.start, w.hz, w.rms[0], w.rms[1]))
            .collect();
        heard.join(", ")
    };
    for h in heard {
        match *h {
            Heard::Tones(tones) => {
                for &(hz, amplitude, need) in tones {
                    let n = tone_windows_at(&ws, hz, amplitude);
                    if n < need {
                        return Err(format!(
                            "sound boot ({how}): {n} windows of {hz} Hz at amplitude {amplitude} in the recording, wanted {need}; heard: {}",
                            what_was_heard()
                        ));
                    }
                    eprintln!("==> heard {hz} Hz at amplitude {amplitude} in {n} windows of 100 ms ({how})");
                }
            }
            Heard::Chord(hzs, amplitude, need) => {
                let n = chord_windows(&rec, hzs, amplitude);
                if n < need {
                    let levels: Vec<String> = hzs
                        .iter()
                        .map(|&hz| {
                            let l: Vec<String> =
                                tone_levels(&rec, 0.1, hz, 0).iter().map(|v| format!("{v:.0}")).collect();
                            format!("{hz} Hz: {}", l.join(" "))
                        })
                        .collect();
                    return Err(format!(
                        "sound boot ({how}): {n} windows with {hzs:?} together at amplitude {amplitude}, wanted {need}; levels: {}",
                        levels.join("; ")
                    ));
                }
                eprintln!("==> heard {hzs:?} together at amplitude {amplitude} in {n} windows of 100 ms ({how})");
            }
        }
    }
    eprintln!(
        "==> the recording: {} Hz, {} channels, {:.1} s",
        rec.rate,
        rec.channels,
        rec.samples.len() as f64 / rec.channels as f64 / rec.rate as f64
    );
    Ok(())
}

/// The kernel's own sound tests with `device` attached.
fn kernel_tests(kernel: &Path, syms: &Arc<Symbols>, device: Device) -> Result {
    eprintln!("==> boot test: the kernel's sound tests with {} attached", device.name());
    let wav = root().join("target/poly-sound-ktest.wav");
    let mut args = vec!["-audiodev".to_string(), format!("wav,id=snd0,path={}", wav.display())];
    args.extend(device.qemu_args());
    let b = boot_with(
        kernel,
        syms,
        DEFAULT_MEMORY,
        "poly.qemu_exit poly.test=sound::",
        Devices::None,
        &args,
        None,
        |_| {},
    )?;
    b.tests(&b.output)?;
    if b.status != Some(QEMU_SUCCESS) || !b.output.contains(PASS_MARKER) || !b.output.contains(device.card_line()) {
        return Err(format!("the kernel's sound tests with {}: QEMU status {:?}", device.name(), b.status));
    }
    Ok(())
}

/// Does this QEMU have Intel HDA and its duplex codec?
fn qemu_has_hda() -> bool {
    Command::new("qemu-system-x86_64").args(["-device", "help"]).output().is_ok_and(|o| {
        let s = String::from_utf8_lossy(&o.stdout);
        s.contains("\"intel-hda\"") && s.contains("\"hda-duplex\"")
    })
}

/// The sound boot tests.
pub fn boot_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    // POLY_SOUND_ONLY=virtio or =hda runs one device's boots (for work on
    // one driver).
    let only = std::env::var("POLY_SOUND_ONLY").unwrap_or_default();
    let virtio = qemu_has_virtio_sound() && only != "hda";
    let hda = qemu_has_hda() && only != "virtio";
    if !virtio && !hda {
        eprintln!("==> skipped: this QEMU has neither virtio-sound-pci nor intel-hda");
        return Ok(());
    }
    if virtio {
        kernel_tests(kernel, syms, Device::Virtio)?;
    }
    if hda {
        kernel_tests(kernel, syms, Device::Hda)?;
    }
    let Some(elf) = build()? else { return Ok(()) };
    let image =
        initramfs::write(&root(), "poly-sound-initramfs.cpio", &initramfs::programs_image(&[("pcm-tone", &elf)]))?;
    const DEFAULT_TONES: &[(f64, f64, usize)] = &[(440.0, AMPLITUDE, 10), (1000.0, AMPLITUDE, 10)];
    if virtio {
        eprintln!(
            "==> boot test: /bin/pcm-tone plays 440 Hz and 1000 Hz through ALSA on virtio-sound (MSI-X), recorded by QEMU"
        );
        // 1.5 s each: at least 1 s of each tone heard intact.
        boot_and_listen(kernel, syms, &image, Device::Virtio, &[], &[Heard::Tones(DEFAULT_TONES)])?;
        eprintln!("==> boot test: the device polled (no MSI-X): 660 Hz in U8 at 22.05 kHz, 880 Hz in FLOAT at 48 kHz");
        let tones = ["22050:660:1000::u8", "48000:880:1000:w:float"];
        const POLLED: &[(f64, f64, usize)] = &[(660.0, AMPLITUDE, 6), (880.0, AMPLITUDE, 6)];
        boot_and_listen(kernel, syms, &image, Device::VirtioPolled, &tones, &[Heard::Tones(POLLED)])?;
        eprintln!(
            "==> boot test: mmap at master volume 50 (523 Hz at half level), then two processes at once (440 + 660 Hz)"
        );
        const HALF: &[(f64, f64, usize)] = &[(523.0, AMPLITUDE / 2.0, 6)];
        boot_and_listen(
            kernel,
            syms,
            &image,
            Device::Virtio,
            &["mmap:48000:523:1000:50", "mix:440:660:1500"],
            &[Heard::Tones(HALF), Heard::Chord(&[440.0, 660.0], AMPLITUDE / 2.0, 8)],
        )?;
        eprintln!("==> boot test: capture from /dev/snd/pcmC0D0c (QEMU's none backend: silence in real time)");
        boot_and_listen(kernel, syms, &image, Device::Virtio, &["capture:48000:1000", "capture:22050:500"], &[])?;
    }
    if hda {
        eprintln!("==> boot test: /bin/pcm-tone plays 440 Hz and 1000 Hz on Intel HDA, recorded by QEMU");
        boot_and_listen(kernel, syms, &image, Device::Hda, &[], &[Heard::Tones(DEFAULT_TONES)])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, f: impl Fn(usize) -> (i16, i16), frames: usize, data_len: Option<u32>) -> Vec<u8> {
        let mut data = Vec::new();
        for i in 0..frames {
            let (l, r) = f(i);
            data.extend_from_slice(&l.to_le_bytes());
            data.extend_from_slice(&r.to_le_bytes());
        }
        let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.unwrap_or(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        b
    }

    fn sine(rate: u32, hz: f64, amp: f64) -> impl Fn(usize) -> (i16, i16) {
        move |i| {
            let s = (amp * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin()).round() as i16;
            (s, s)
        }
    }

    #[test]
    fn a_recorded_tone_is_recognised_by_frequency_and_level() {
        let b = wav(44100, sine(44100, 440.0, AMPLITUDE), 44100, None);
        let w = parse_wav(&b).unwrap();
        assert_eq!((w.rate, w.channels, w.samples.len()), (44100, 2, 88200));
        let ws = windows(&w, 0.1);
        assert_eq!(ws.len(), 10);
        assert!(ws.iter().all(|w| (w.hz - 440.0).abs() < 1.0), "{ws:?}");
        assert_eq!(tone_windows(&ws, 440.0), 10);
        assert_eq!(tone_windows(&ws, 1000.0), 0);
        // Too quiet, or one channel silent: not the tone.
        let quiet = parse_wav(&wav(44100, sine(44100, 440.0, AMPLITUDE / 4.0), 44100, None)).unwrap();
        assert_eq!(tone_windows(&windows(&quiet, 0.1), 440.0), 0);
        let mono = parse_wav(&wav(44100, |i| (sine(44100, 440.0, AMPLITUDE)(i).0, 0), 44100, None)).unwrap();
        assert_eq!(tone_windows(&windows(&mono, 0.1), 440.0), 0);
    }

    #[test]
    fn two_tones_at_once_are_found_by_their_levels() {
        let a = sine(44100, 440.0, AMPLITUDE / 2.0);
        let b = sine(44100, 660.0, AMPLITUDE / 2.0);
        let both = parse_wav(&wav(44100, |i| (a(i).0 + b(i).0, a(i).1 + b(i).1), 44100, None)).unwrap();
        let l = tone_levels(&both, 0.1, 440.0, 0);
        assert_eq!(l.len(), 10);
        assert!(l.iter().all(|v| (v - AMPLITUDE / 2.0).abs() < 50.0), "{l:?}");
        assert_eq!(chord_windows(&both, &[440.0, 660.0], AMPLITUDE / 2.0), 10);
        // One tone alone is not the chord; nor is the pair at full level.
        let one = parse_wav(&wav(44100, &a, 44100, None)).unwrap();
        assert_eq!(chord_windows(&one, &[440.0, 660.0], AMPLITUDE / 2.0), 0);
        assert_eq!(chord_windows(&both, &[440.0, 660.0], AMPLITUDE), 0);
        // Half the level is found by tone_windows_at, not by tone_windows.
        let ws = windows(&one, 0.1);
        assert_eq!((tone_windows(&ws, 440.0), tone_windows_at(&ws, 440.0, AMPLITUDE / 2.0)), (0, 10));
    }

    #[test]
    fn an_unfinished_recording_runs_to_the_end_of_the_file() {
        let b = wav(48000, sine(48000, 1000.0, AMPLITUDE), 9600, Some(0));
        let w = parse_wav(&b).unwrap();
        assert_eq!(w.samples.len(), 19200);
        assert_eq!(tone_windows(&windows(&w, 0.1), 1000.0), 2);
        assert!(parse_wav(b"RIFF").is_err());
    }
}
