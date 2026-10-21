//! Which QEMU accelerator the boots run on: KVM when `/dev/kvm` can be
//! opened (much faster: the guest runs on the host CPU), QEMU's emulator
//! (TCG) otherwise. `POLY_QEMU_ACCEL` overrides it: `tcg` always emulates,
//! `kvm` requires KVM (an error without it), `auto` (or unset) chooses.
//!
//! The CPU model is the same on both: QEMU's default (`qemu64`) unless a
//! boot asks for another with `-cpu`, so the guest sees the same features
//! whichever runs it. Under KVM, QEMU adds x2APIC (in-kernel APIC) and the
//! guest's TSC is the host's (the kernel treats a hypervisor's TSC as
//! constant-rate either way, ADR 0072); `-cpu max` under KVM is the host
//! CPU's feature set, under TCG everything TCG emulates. CI runs the
//! self-test boots under TCG as well (ADR 0077), so code paths only one of
//! them takes stay tested.

use std::sync::{Once, OnceLock};

/// The environment variable that chooses the accelerator.
pub const ENV: &str = "POLY_QEMU_ACCEL";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accel {
    Kvm,
    Tcg,
}

impl Accel {
    /// QEMU's arguments for it.
    pub fn qemu_args(self) -> [&'static str; 2] {
        match self {
            Accel::Kvm => ["-accel", "kvm"],
            Accel::Tcg => ["-accel", "tcg"],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Accel::Kvm => "KVM",
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

/// The accelerator for `POLY_QEMU_ACCEL=<var>` (None: unset) given whether
/// KVM is usable, and a line saying why.
pub fn choose(var: Option<&str>, kvm: std::result::Result<(), String>) -> std::result::Result<(Accel, String), String> {
    match var.map(str::trim) {
        None | Some("" | "auto") => Ok(match kvm {
            Ok(()) => (Accel::Kvm, "KVM (/dev/kvm is usable; POLY_QEMU_ACCEL=tcg emulates)".into()),
            Err(why) => (Accel::Tcg, format!("TCG, QEMU's emulator (no KVM: {why})")),
        }),
        Some("tcg") => Ok((Accel::Tcg, "TCG, QEMU's emulator (POLY_QEMU_ACCEL=tcg)".into())),
        Some("kvm") => match kvm {
            Ok(()) => Ok((Accel::Kvm, "KVM (POLY_QEMU_ACCEL=kvm)".into())),
            Err(why) => Err(format!("{ENV}=kvm, but KVM is not usable: {why}")),
        },
        Some(other) => Err(format!("{ENV}={other}: expected kvm, tcg or auto")),
    }
}

static CHOSEN: OnceLock<(Accel, String)> = OnceLock::new();

/// Choose the accelerator from the environment (once; `main` calls it
/// first, so a bad `POLY_QEMU_ACCEL` stops every command).
pub fn init() -> std::result::Result<(), String> {
    if CHOSEN.get().is_none() {
        let var = std::env::var(ENV).ok();
        let chosen = choose(var.as_deref(), kvm_usable())?;
        let _ = CHOSEN.set(chosen);
    }
    Ok(())
}

/// The accelerator chosen by [`init`] (TCG if it was never called). The
/// first call says which it is.
pub fn get() -> Accel {
    static SAID: Once = Once::new();
    let (accel, why) = CHOSEN.get_or_init(|| (Accel::Tcg, "TCG, QEMU's emulator".into()));
    SAID.call_once(|| eprintln!("==> QEMU accelerator: {why}"));
    *accel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_kvm_when_usable_unless_told_otherwise() {
        let yes = || Ok(());
        let no = || Err("/dev/kvm: No such file or directory".to_string());
        for var in [None, Some(""), Some("auto"), Some(" auto ")] {
            assert_eq!(choose(var, yes()).unwrap().0, Accel::Kvm, "{var:?}");
            assert_eq!(choose(var, no()).unwrap().0, Accel::Tcg, "{var:?}");
        }
        assert_eq!(choose(Some("tcg"), yes()).unwrap().0, Accel::Tcg);
        assert_eq!(choose(Some("tcg"), no()).unwrap().0, Accel::Tcg);
        assert_eq!(choose(Some("kvm"), yes()).unwrap().0, Accel::Kvm);
        assert!(choose(Some("kvm"), no()).unwrap_err().contains("not usable"));
        assert!(choose(Some("hvf"), yes()).is_err());
        assert_eq!(Accel::Kvm.qemu_args(), ["-accel", "kvm"]);
        assert_eq!(Accel::Tcg.qemu_args(), ["-accel", "tcg"]);
    }
}
