//! Which QEMU accelerator the boots run on: the host's hypervisor when QEMU
//! can use it (much faster: the guest runs on the host CPU), QEMU's
//! emulator (TCG) otherwise. On Linux that is KVM (`/dev/kvm` can be
//! opened); on Windows the Windows Hypervisor Platform (WHPX, the optional
//! Windows feature of that name; QEMU is asked once whether it can start
//! it). `POLY_QEMU_ACCEL` overrides it: `tcg` always emulates, `kvm` or
//! `whpx` requires that hypervisor (an error without it), `auto` (or unset)
//! chooses.
//!
//! The CPU model is the same on all: QEMU's default (`qemu64`) unless a
//! boot asks for another with `-cpu`, so the guest sees the same features
//! whichever runs it. Under KVM, QEMU adds x2APIC (in-kernel APIC) and the
//! guest's TSC is the host's (the kernel treats a hypervisor's TSC as
//! constant-rate either way, ADR 0072); `-cpu max` under KVM is the host
//! CPU's feature set, under TCG everything TCG emulates. CI runs the
//! self-test boots under TCG as well (ADR 0077), so code paths only one of
//! them takes stay tested.
//!
//! `POLY_QEMU_CPU=<model>` passes `-cpu <model>` to every boot that does not
//! name a model itself. It is for experiments: `qemu64` has no invariant TSC
//! (CPUID.80000007H:EDX bit 8), which real machines have and programs test
//! for with CPUID (Chrome for Windows takes `timeGetTime` instead of
//! `QueryPerformanceCounter` without it, ADR 0155);
//! `POLY_QEMU_CPU=host,+invtsc` under KVM shows the guest the host's.

use std::sync::{Once, OnceLock};

/// The environment variable that chooses the accelerator.
pub const ENV: &str = "POLY_QEMU_ACCEL";

/// The environment variable that names QEMU's CPU model.
pub const CPU_ENV: &str = "POLY_QEMU_CPU";

/// QEMU's arguments for `POLY_QEMU_CPU=<var>` (None: unset): `-cpu <model>`,
/// or nothing (QEMU's default model) when it is unset or empty. They come
/// before a boot's own arguments, so a boot that names a model keeps it
/// (QEMU takes the last `-cpu`).
pub fn cpu_args(var: Option<&str>) -> Vec<String> {
    match var.map(str::trim) {
        Some(model) if !model.is_empty() => vec!["-cpu".to_string(), model.to_string()],
        _ => Vec::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accel {
    Kvm,
    Whpx,
    Tcg,
}

impl Accel {
    /// QEMU's arguments for it.
    pub fn qemu_args(self) -> [&'static str; 2] {
        match self {
            Accel::Kvm => ["-accel", "kvm"],
            Accel::Whpx => ["-accel", "whpx"],
            Accel::Tcg => ["-accel", "tcg"],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Accel::Kvm => "KVM",
            Accel::Whpx => "WHPX",
            Accel::Tcg => "TCG",
        }
    }
}

/// Whether `/dev/kvm` can be opened for reading and writing (what QEMU
/// needs), or why not.
pub fn kvm_usable() -> std::result::Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("KVM is Linux only".into());
    }
    std::fs::OpenOptions::new().read(true).write(true).open("/dev/kvm").map(drop).map_err(|e| format!("/dev/kvm: {e}"))
}

/// Whether QEMU can start the Windows Hypervisor Platform, or why not: a
/// QEMU with a machine that never runs (`-S`; WHPX needs an x86 one). QEMU
/// sets the accelerator up before anything else and exits at once when it
/// cannot, so one still running after [`WHPX_PROBE`] has it (and is
/// killed). Windows' QEMU reads no monitor commands from a pipe, so it
/// cannot be told to quit.
pub fn whpx_usable() -> std::result::Result<(), String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Instant;
    if !cfg!(windows) {
        return Err("WHPX is Windows only".into());
    }
    let mut child = Command::new("qemu-system-x86_64")
        .args(["-accel", "whpx", "-machine", "q35", "-S", "-display", "none", "-nodefaults"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("qemu-system-x86_64: {e}"))?;
    let deadline = Instant::now() + WHPX_PROBE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(());
            }
        }
    }
    let mut err = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut err);
    }
    let line = err.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("QEMU could not start it").trim();
    Err(format!("{line} (turn on the Windows feature \"Windows Hypervisor Platform\")"))
}

/// How long a QEMU asked for WHPX must keep running to count as having it.
const WHPX_PROBE: std::time::Duration = std::time::Duration::from_millis(1500);

/// The host's hypervisor and whether QEMU can use it: KVM on Linux, WHPX
/// on Windows.
fn host_hypervisor() -> (Accel, std::result::Result<(), String>) {
    if cfg!(windows) { (Accel::Whpx, whpx_usable()) } else { (Accel::Kvm, kvm_usable()) }
}

/// The accelerator for `POLY_QEMU_ACCEL=<var>` (None: unset) given the
/// host's hypervisor (`host`) and whether it is usable, and a line saying
/// why.
pub fn choose(
    var: Option<&str>,
    host: Accel,
    usable: std::result::Result<(), String>,
) -> std::result::Result<(Accel, String), String> {
    let name = host.name();
    match var.map(str::trim) {
        None | Some("" | "auto") => Ok(match usable {
            Ok(()) => (host, format!("{name}, the host's hypervisor (POLY_QEMU_ACCEL=tcg emulates)")),
            Err(why) => (Accel::Tcg, format!("TCG, QEMU's emulator, much slower (no {name}: {why})")),
        }),
        Some("tcg") => Ok((Accel::Tcg, "TCG, QEMU's emulator (POLY_QEMU_ACCEL=tcg)".into())),
        Some(v @ ("kvm" | "whpx")) if !v.eq_ignore_ascii_case(name) => {
            Err(format!("{ENV}={v}, but this host's hypervisor is {name}"))
        }
        Some(v @ ("kvm" | "whpx")) => match usable {
            Ok(()) => Ok((host, format!("{name} ({ENV}={v})"))),
            Err(why) => Err(format!("{ENV}={v}, but {name} is not usable: {why}")),
        },
        Some(other) => Err(format!("{ENV}={other}: expected kvm, whpx, tcg or auto")),
    }
}

static CHOSEN: OnceLock<(Accel, String)> = OnceLock::new();

/// Check `POLY_QEMU_ACCEL`'s value (`main` calls it first, so a bad value
/// stops every command). Which accelerator is usable is found out on the
/// first boot ([`get`]): asking QEMU about WHPX takes a moment, which
/// commands that boot nothing should not pay.
pub fn init() -> std::result::Result<(), String> {
    match std::env::var(ENV).ok().as_deref().map(str::trim) {
        None | Some("" | "auto" | "tcg" | "kvm" | "whpx") => Ok(()),
        Some(other) => Err(format!("{ENV}={other}: expected kvm, whpx, tcg or auto")),
    }
}

/// The accelerator to boot on, chosen on the first call (which says which
/// it is). `POLY_QEMU_ACCEL=kvm|whpx` without that hypervisor stops xtask.
pub fn get() -> Accel {
    static SAID: Once = Once::new();
    let (accel, why) = CHOSEN.get_or_init(|| {
        let var = std::env::var(ENV).ok();
        let (host, usable) = match var.as_deref().map(str::trim) {
            Some("tcg") => (Accel::Tcg, Err(String::new())),
            _ => host_hypervisor(),
        };
        choose(var.as_deref(), host, usable).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1)
        })
    });
    SAID.call_once(|| eprintln!("==> QEMU accelerator: {why}"));
    *accel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cpu_model_is_passed_only_when_named() {
        assert!(cpu_args(None).is_empty());
        assert!(cpu_args(Some("")).is_empty());
        assert!(cpu_args(Some("  ")).is_empty());
        assert_eq!(cpu_args(Some("host,+invtsc")), ["-cpu", "host,+invtsc"]);
        assert_eq!(cpu_args(Some(" max ")), ["-cpu", "max"]);
    }

    #[test]
    fn chooses_kvm_when_usable_unless_told_otherwise() {
        let yes = || Ok(());
        let no = || Err("/dev/kvm: No such file or directory".to_string());
        let kvm = Accel::Kvm;
        for var in [None, Some(""), Some("auto"), Some(" auto ")] {
            assert_eq!(choose(var, kvm, yes()).unwrap().0, Accel::Kvm, "{var:?}");
            assert_eq!(choose(var, kvm, no()).unwrap().0, Accel::Tcg, "{var:?}");
        }
        assert_eq!(choose(Some("tcg"), kvm, yes()).unwrap().0, Accel::Tcg);
        assert_eq!(choose(Some("tcg"), kvm, no()).unwrap().0, Accel::Tcg);
        assert_eq!(choose(Some("kvm"), kvm, yes()).unwrap().0, Accel::Kvm);
        assert!(choose(Some("kvm"), kvm, no()).unwrap_err().contains("not usable"));
        assert!(choose(Some("whpx"), kvm, yes()).unwrap_err().contains("hypervisor is KVM"));
        assert!(choose(Some("hvf"), kvm, yes()).is_err());
        assert_eq!(Accel::Kvm.qemu_args(), ["-accel", "kvm"]);
        assert_eq!(Accel::Tcg.qemu_args(), ["-accel", "tcg"]);
    }

    #[test]
    fn chooses_whpx_on_windows_when_usable() {
        let whpx = Accel::Whpx;
        assert_eq!(choose(None, whpx, Ok(())).unwrap().0, Accel::Whpx);
        let (accel, why) = choose(None, whpx, Err("WHPX: No accelerator found".into())).unwrap();
        assert_eq!(accel, Accel::Tcg);
        assert!(why.contains("no WHPX"), "{why}");
        assert_eq!(choose(Some("whpx"), whpx, Ok(())).unwrap().0, Accel::Whpx);
        assert!(choose(Some("whpx"), whpx, Err("off".into())).unwrap_err().contains("not usable"));
        assert!(choose(Some("kvm"), whpx, Ok(())).unwrap_err().contains("hypervisor is WHPX"));
        assert_eq!(choose(Some("tcg"), whpx, Ok(())).unwrap().0, Accel::Tcg);
        assert_eq!(Accel::Whpx.qemu_args(), ["-accel", "whpx"]);
    }
}
