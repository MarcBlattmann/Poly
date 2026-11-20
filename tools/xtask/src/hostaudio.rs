//! The desktop's sound card (batch A0 of `docs/windows/audio-plan.md`): `cargo xtask desktop`
//! gives the guest a sound device played through the host's speakers, so Poly's sound stack
//! (ADR 0104, ADR 0109) is heard (Linux Chromium plays through ALSA on it as it is).
//!
//! - The device: `virtio-sound-pci` when this QEMU has it (Poly's first and fullest driver:
//!   playback and capture), else `intel-hda` with `hda-output` (playback, ADR 0109).
//! - The host's side, QEMU's audio backend (QEMU documentation, "Audio options"): `dsound` on
//!   Windows, `pa` on Linux (else `pipewire`, else `alsa`), `coreaudio` on macOS, whichever of them
//!   this QEMU lists in `-audiodev help`.
//! - `--no-sound`: no device. `--record-sound=FILE` (or `--record-sound FILE`): QEMU's `wav`
//!   backend writes what the guest played to `FILE` instead (for checks without speakers, as the
//!   `sound` boot test does).
//! - A host whose backend cannot start (no sound device, no sound server: a container) still
//!   boots. QEMU itself keeps going then, with warnings and a silent device; a short probe finds
//!   out before the boot instead, and the guest keeps its card on the `none` backend, which plays
//!   into nothing without a word from QEMU. One line of the boot says so.

use std::path::PathBuf;

/// The host's operating system, as far as the backend choice cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Windows,
    Linux,
    MacOs,
    Other,
}

impl Host {
    pub fn this() -> Host {
        if cfg!(windows) {
            Host::Windows
        } else if cfg!(target_os = "linux") {
            Host::Linux
        } else if cfg!(target_os = "macos") {
            Host::MacOs
        } else {
            Host::Other
        }
    }

    /// QEMU's audio backends for this host, best first.
    fn backends(self) -> &'static [&'static str] {
        match self {
            Host::Windows => &["dsound"],
            Host::Linux => &["pa", "pipewire", "alsa"],
            Host::MacOs => &["coreaudio"],
            Host::Other => &[],
        }
    }
}

/// The guest's sound device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    /// `virtio-sound-pci` (ADR 0104).
    Virtio,
    /// `intel-hda` with an output codec (ADR 0109).
    Hda,
}

/// Where the guest's sound goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Backend {
    /// A host backend (`dsound`, `pa`, ...): the speakers.
    Host(&'static str),
    /// QEMU's `wav` backend: a file.
    Record(PathBuf),
    /// Nowhere (`none`): the card is there, nothing is heard.
    Silent,
}

/// The sound setup of a desktop boot (`None` from [`choose`]: no device).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub device: Device,
    pub backend: Backend,
}

/// The id the device and its backend are joined by.
const ID: &str = "polysnd";

/// The flag that turns the device off, and the one that records instead.
pub const NO_SOUND: &str = "--no-sound";
pub const RECORD_SOUND: &str = "--record-sound";

/// The file `--record-sound=FILE` or `--record-sound FILE` names, if given.
pub fn record_file(args: &[String]) -> Result<Option<PathBuf>, String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix("--record-sound=") {
            return non_empty(v).map(Some);
        }
        if a == RECORD_SOUND {
            let v = it.next().ok_or("--record-sound needs a file: --record-sound=FILE")?;
            return non_empty(v).map(Some);
        }
    }
    Ok(None)
}

fn non_empty(v: &str) -> Result<PathBuf, String> {
    if v.is_empty() { Err("--record-sound needs a file: --record-sound=FILE".into()) } else { Ok(PathBuf::from(v)) }
}

/// Whether `a` is the file of a `--record-sound FILE` pair (the argument after the flag), which
/// is not a kernel argument.
pub fn is_record_file(args: &[String], i: usize) -> bool {
    i > 0 && args[i - 1] == RECORD_SOUND
}

/// The backends a QEMU lists in `-audiodev help` ("Available audio drivers:" and a name per
/// line).
pub fn listed_backends(help: &str) -> Vec<String> {
    help.lines()
        .skip_while(|l| !l.to_ascii_lowercase().contains("audio drivers"))
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains(' '))
        .map(str::to_string)
        .collect()
}

/// Decide the setup from the arguments, the host, the backends this QEMU lists and whether it has
/// virtio-sound. `None`: no device (`--no-sound`).
pub fn choose(args: &[String], host: Host, listed: &[String], has_virtio: bool) -> Result<Option<Plan>, String> {
    if args.iter().any(|a| a == NO_SOUND) {
        if record_file(args)?.is_some() {
            return Err("--no-sound and --record-sound: choose one".into());
        }
        return Ok(None);
    }
    let device = if has_virtio { Device::Virtio } else { Device::Hda };
    let backend = match record_file(args)? {
        Some(file) => Backend::Record(file),
        None => host
            .backends()
            .iter()
            .find(|b| listed.iter().any(|l| l == *b))
            .map_or(Backend::Silent, |b| Backend::Host(b)),
    };
    Ok(Some(Plan { device, backend }))
}

impl Plan {
    /// QEMU's arguments for the plan.
    pub fn qemu_args(&self) -> Vec<String> {
        let audiodev = match &self.backend {
            Backend::Host(b) => format!("{b},id={ID}"),
            // The wav backend records 44.1 kHz stereo S16; it has no input (QEMU says so once).
            Backend::Record(f) => format!("wav,id={ID},path={}", f.display()),
            Backend::Silent => format!("none,id={ID}"),
        };
        let mut v = vec!["-audiodev".to_string(), audiodev];
        match self.device {
            Device::Virtio => v.extend(["-device".to_string(), format!("virtio-sound-pci,audiodev={ID}")]),
            Device::Hda => v.extend([
                "-device".to_string(),
                "intel-hda".to_string(),
                "-device".to_string(),
                format!("hda-output,audiodev={ID}"),
            ]),
        }
        v
    }

    /// One line for the boot's log.
    pub fn describe(&self) -> String {
        let device = match self.device {
            Device::Virtio => "virtio-sound",
            Device::Hda => "Intel HDA",
        };
        match &self.backend {
            Backend::Host(b) => format!("sound: {device} through the host's speakers ({b}; {NO_SOUND} turns it off)"),
            Backend::Record(f) => format!("sound: {device}, recorded to {}", f.display()),
            Backend::Silent => format!("sound: {device}, heard nowhere (no host audio backend in this QEMU)"),
        }
    }
}

/// How long the probe's QEMU runs before its messages are read.
const PROBE: std::time::Duration = std::time::Duration::from_millis(1200);

/// Whether QEMU can play through the plan's host backend, or why not. QEMU does not stop when a
/// backend cannot start: it says `audio: Could not ...` and keeps the device, which then plays
/// nowhere (and says so again in the boot's output). So a QEMU with the device and the backend
/// and no machine running (`-S`) is started, stopped after [`PROBE`], and what it said is read.
pub fn probe(qemu: &str, plan: &Plan) -> Result<(), String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Instant;
    let mut child = Command::new(qemu)
        .args(["-machine", "q35", "-S", "-display", "none", "-nodefaults"])
        .args(plan.qemu_args())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{qemu}: {e}"))?;
    let deadline = Instant::now() + PROBE;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    let mut err = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut err);
    }
    match output_failure(&err) {
        Some(line) => Err(line),
        None => Ok(()),
    }
}

/// The line of QEMU's messages saying that sound cannot be played: the driver did not start, or
/// the output voice has no backend. A missing input (no microphone) does not count.
pub fn output_failure(messages: &str) -> Option<String> {
    messages
        .lines()
        .find(|l| {
            l.contains("Could not init `")
                || l.contains("Unknown audio driver")
                || (l.contains("Could not create a backend for voice") && l.contains(".out"))
        })
        .map(|l| l.trim().to_string())
}

/// The setup for a desktop boot from its arguments, asking this host's QEMU (`qemu`) what it
/// has; a host backend that cannot start becomes [`Backend::Silent`] with a line saying why.
pub fn for_desktop(qemu: &str, args: &[String]) -> Result<Option<Plan>, String> {
    use std::process::Command;
    let help = |what: &str| {
        Command::new(qemu)
            .args([what, "help"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let listed = listed_backends(&help("-audiodev"));
    let has_virtio = help("-device").contains("\"virtio-sound-pci\"");
    let Some(mut plan) = choose(args, Host::this(), &listed, has_virtio)? else { return Ok(None) };
    if let Backend::Host(b) = plan.backend
        && let Err(why) = probe(qemu, &plan)
    {
        crate::progress::note(&format!(
            "sound: the host's {b} backend does not start ({why}): the guest's card plays into nothing"
        ));
        plan.backend = Backend::Silent;
    }
    Ok(Some(plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    const WIN_HELP: &str = "Available audio drivers:\nnone\ndbus\ndsound\njack\nsdl\nspice\nwav\n";

    #[test]
    fn the_backends_this_qemu_lists() {
        assert_eq!(listed_backends(WIN_HELP), ["none", "dbus", "dsound", "jack", "sdl", "spice", "wav"]);
        assert!(listed_backends("").is_empty());
    }

    #[test]
    fn each_host_takes_its_own_backend() {
        let listed = listed_backends(WIN_HELP);
        let p = choose(&[], Host::Windows, &listed, true).unwrap().unwrap();
        assert_eq!(p, Plan { device: Device::Virtio, backend: Backend::Host("dsound") });
        let linux = ["none".to_string(), "alsa".to_string(), "pipewire".to_string(), "pa".to_string()];
        assert_eq!(choose(&[], Host::Linux, &linux, true).unwrap().unwrap().backend, Backend::Host("pa"));
        assert_eq!(choose(&[], Host::Linux, &linux[..3], true).unwrap().unwrap().backend, Backend::Host("pipewire"));
        let mac = ["coreaudio".to_string()];
        assert_eq!(choose(&[], Host::MacOs, &mac, false).unwrap().unwrap().backend, Backend::Host("coreaudio"));
        // None of the host's backends: the card stays, heard nowhere.
        assert_eq!(choose(&[], Host::Linux, &listed, true).unwrap().unwrap().backend, Backend::Silent);
    }

    #[test]
    fn only_a_failed_output_means_no_sound() {
        let input_only = "alsa: Could not initialize ADC
audio: Could not create a backend for voice `virtio-sound.in'
";
        assert_eq!(output_failure(input_only), None);
        let no_out = "audio: Could not create a backend for voice `virtio-sound.out'
";
        assert!(output_failure(no_out).is_some());
        assert!(
            output_failure(
                "qemu: Unknown audio driver `pa'.
"
            )
            .is_some()
        );
        assert!(
            output_failure(
                "audio: Could not init `pa' audio driver
"
            )
            .is_some()
        );
        assert_eq!(output_failure(""), None);
    }

    #[test]
    fn hda_without_virtio_sound() {
        let p = choose(&[], Host::Windows, &listed_backends(WIN_HELP), false).unwrap().unwrap();
        assert_eq!(p.device, Device::Hda);
        assert_eq!(
            p.qemu_args(),
            args(&["-audiodev", "dsound,id=polysnd", "-device", "intel-hda", "-device", "hda-output,audiodev=polysnd"])
        );
    }

    #[test]
    fn off_and_recording() {
        assert_eq!(choose(&args(&["--no-sound"]), Host::Windows, &[], true).unwrap(), None);
        let p = choose(&args(&["--record-sound=out.wav"]), Host::Windows, &[], true).unwrap().unwrap();
        assert_eq!(
            p.qemu_args(),
            args(&["-audiodev", "wav,id=polysnd,path=out.wav", "-device", "virtio-sound-pci,audiodev=polysnd"])
        );
        let two = args(&["--release", "--record-sound", "a.wav", "poly.x=1"]);
        assert_eq!(choose(&two, Host::Linux, &[], true).unwrap().unwrap().backend, Backend::Record("a.wav".into()));
        assert!(is_record_file(&two, 2) && !is_record_file(&two, 3));
        assert!(choose(&args(&["--record-sound"]), Host::Linux, &[], true).is_err());
        assert!(choose(&args(&["--record-sound="]), Host::Linux, &[], true).is_err());
        assert!(choose(&args(&["--no-sound", "--record-sound=x.wav"]), Host::Linux, &[], true).is_err());
        let silent = Plan { device: Device::Virtio, backend: Backend::Silent };
        assert_eq!(silent.qemu_args()[1], "none,id=polysnd");
    }
}
