//! The input boot tests (roadmap Phase 6, ADR 0103): keys, motion, the
//! wheel and clicks sent through QEMU's QMP `input-send-event` reach a
//! Linux program reading `/dev/input/event*`.
//!
//! `/bin/evtest` (`tools/xtask/programs/evtest.rs`, no C library) runs as
//! the first process: it describes every event device with the `EVIOC*`
//! ioctls, checks a grab and an LED write, prints `evtest: ready` and then
//! every event it reads, until a Q key press. Two boots:
//!
//! 1. **PS/2**: QEMU's i8042 keyboard and mouse only. Keys must arrive on
//!    the PS/2 keyboard's device, motion, the wheel and a click on the PS/2
//!    mouse's (an IntelliMouse with a wheel, detected by the kernel).
//! 2. **virtio**: `virtio-keyboard-pci`, `virtio-mouse-pci` and
//!    `virtio-tablet-pci` as well. An absolute position must arrive on the
//!    tablet, keys and relative motion on the devices QEMU routes them to;
//!    the kernel test `virtio_input::devices_found` checks all three were
//!    brought up.
//!
//! The QMP event names are QEMU's (its QMP reference: `input-send-event`,
//! `InputEvent`, `QKeyCode`, `InputButton`, `InputAxis`); absolute axes run
//! from 0 to 0x7fff.

use super::{Devices, QEMU_SUCCESS, Result, boot_with, initramfs, programs, root, screen};
use crate::symbols::Symbols;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// What `/bin/evtest` prints once it waits for input, and when it is done.
const READY: &str = "evtest: ready";
const DONE: &str = "evtest: done";

/// One `InputEvent` of `input-send-event`, as JSON.
fn key(qcode: &str, down: bool) -> String {
    format!(r#"{{"type": "key", "data": {{"down": {down}, "key": {{"type": "qcode", "data": "{qcode}"}}}}}}"#)
}

fn button(name: &str, down: bool) -> String {
    format!(r#"{{"type": "btn", "data": {{"down": {down}, "button": "{name}"}}}}"#)
}

fn axis(kind: &str, name: &str, value: i32) -> String {
    format!(r#"{{"type": "{kind}", "data": {{"axis": "{name}", "value": {value}}}}}"#)
}

/// Send each group of events with one `input-send-event`, a little apart.
/// The last group ends the guest program, and with it QEMU, which may not
/// stay to answer: its errors do not count.
fn send(socket: &Path, groups: &[Vec<String>]) -> std::result::Result<(), String> {
    let mut qmp = screen::Qmp::connect(socket, Duration::from_secs(10))?;
    for (i, g) in groups.iter().enumerate() {
        let r = qmp.execute("input-send-event", &format!(r#"{{"events": [{}]}}"#, g.join(", ")));
        if i + 1 < groups.len() {
            r?;
        }
        thread::sleep(Duration::from_millis(150));
    }
    Ok(())
}

/// The keys, motion, wheel and click of both boots, then Q to end.
fn script(tablet: bool) -> Vec<Vec<String>> {
    let mut g = vec![
        vec![key("a", true)],
        vec![key("a", false)],
        vec![axis("rel", "x", 10), axis("rel", "y", 5)],
        vec![button("wheel-up", true)],
        vec![button("wheel-up", false)],
        vec![button("left", true)],
        vec![button("left", false)],
    ];
    if tablet {
        g.push(vec![axis("abs", "x", 16384), axis("abs", "y", 8192)]);
    }
    g.push(vec![key("q", true), key("q", false)]);
    g
}

/// Boot `/bin/evtest` with `devices` (QEMU `-device` names) and the
/// script; returns the output.
fn boot(kernel: &Path, syms: &Arc<Symbols>, image: &Path, devices: &[&str], append: &str) -> Result<String> {
    let socket: PathBuf = std::env::temp_dir().join(format!("poly-input-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let mut qemu_args = vec![
        "-initrd".to_string(),
        image.display().to_string(),
        "-qmp".to_string(),
        format!("unix:{},server=on,wait=off", socket.display()),
    ];
    for d in devices {
        qemu_args.extend(["-device".to_string(), d.to_string()]);
    }
    let tablet = devices.iter().any(|d| d.contains("tablet"));
    let path = socket.clone();
    let mut sent = false;
    let on_line = move |line: &str| {
        if !sent && line.contains(READY) {
            sent = true;
            let path = path.clone();
            thread::spawn(move || {
                if let Err(e) = send(&path, &script(tablet)) {
                    eprintln!("xtask: sending input through QMP: {e}");
                }
            });
        }
    };
    let append = format!("poly.qemu_exit poly.init=/bin/evtest {append}");
    let b = boot_with(kernel, syms, super::DEFAULT_MEMORY, &append, Devices::None, &qemu_args, None, on_line);
    let _ = std::fs::remove_file(&socket);
    let b = b?;
    let ok = b.status == Some(QEMU_SUCCESS)
        && b.output.contains(DONE)
        && b.output.contains("POLY-EXIT: pid=1 code=0")
        && b.output.contains("evtest: grab ok")
        && b.output.contains("evtest: led ok");
    if !ok {
        let failed = b.output.lines().find(|l| l.contains("evtest:") && l.contains("failed")).unwrap_or("");
        return Err(format!("/bin/evtest did not finish its checks (QEMU status {:?}) {failed}", b.status));
    }
    Ok(b.output)
}

/// The event device number `evtest` printed for the device called `name`.
fn device(output: &str, name: &str) -> Result<String> {
    output
        .lines()
        .filter_map(|l| l.split("evtest: ").nth(1))
        .find(|l| l.contains(&format!("\"{name}")))
        .and_then(|l| l.split_whitespace().next())
        .map(String::from)
        .ok_or_else(|| format!("no event device called {name:?}"))
}

/// Whether `evtest` read an event of `kind` and `code` on `dev`, with a
/// value `pred` accepts.
fn got(output: &str, dev: &str, kind: u16, code: u16, pred: impl Fn(i32) -> bool) -> bool {
    let prefix = format!("evtest: {dev} type {kind} code {code} value ");
    output.lines().filter_map(|l| l.split(&prefix).nth(1)).filter_map(|v| v.trim().parse().ok()).any(pred)
}

/// Check every expectation; the error names the ones that failed.
fn expect(checks: &[(&str, bool)]) -> Result {
    let failed: Vec<&str> = checks.iter().filter(|(_, ok)| !ok).map(|(what, _)| *what).collect();
    if failed.is_empty() { Ok(()) } else { Err(format!("input boot test: {}", failed.join("; "))) }
}

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const KEY_A: u16 = 30;
const BTN_LEFT: u16 = 0x110;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_WHEEL: u16 = 8;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;

pub fn boot_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let evtest = programs::evtest()?;
    let image =
        initramfs::write(&root(), "poly-input-initramfs.cpio", &initramfs::programs_image(&[("evtest", &evtest)]))?;

    eprintln!(
        "==> boot test: keys, motion, the wheel and clicks reach /dev/input/event* through the PS/2 keyboard and mouse"
    );
    let out = boot(kernel, syms, &image, &[], "")?;
    let kbd = device(&out, "PS/2 Keyboard")?;
    let mouse = device(&out, "PS/2 IntelliMouse")?;
    expect(&[
        (
            "A pressed and released on the PS/2 keyboard",
            got(&out, &kbd, EV_KEY, KEY_A, |v| v == 1) && got(&out, &kbd, EV_KEY, KEY_A, |v| v == 0),
        ),
        ("motion right on the PS/2 mouse", got(&out, &mouse, EV_REL, REL_X, |v| v > 0)),
        ("motion down on the PS/2 mouse", got(&out, &mouse, EV_REL, REL_Y, |v| v > 0)),
        ("the wheel up on the PS/2 mouse", got(&out, &mouse, EV_REL, REL_WHEEL, |v| v > 0)),
        (
            "a left click on the PS/2 mouse",
            got(&out, &mouse, EV_KEY, BTN_LEFT, |v| v == 1) && got(&out, &mouse, EV_KEY, BTN_LEFT, |v| v == 0),
        ),
    ])?;

    eprintln!("==> boot test: virtio keyboard, mouse and tablet reach /dev/input/event* (virtio-input over MSI-X)");
    let devices = ["virtio-keyboard-pci", "virtio-mouse-pci", "virtio-tablet-pci"];
    let out = boot(kernel, syms, &image, &devices, "")?;
    let tablet = device(&out, "QEMU Virtio Tablet")?;
    let vkbd = device(&out, "QEMU Virtio Keyboard")?;
    let vmouse = device(&out, "QEMU Virtio Mouse")?;
    let (kbd, mouse) = (device(&out, "PS/2 Keyboard")?, device(&out, "PS/2 IntelliMouse")?);
    let keyboards = [kbd.as_str(), vkbd.as_str()];
    let mice = [mouse.as_str(), vmouse.as_str()];
    let interrupts = out.lines().filter(|l| l.starts_with("virtio-input: ") && l.contains("interrupt-driven")).count();
    expect(&[
        ("the three virtio devices interrupt-driven (MSI-X)", interrupts == 3),
        (
            "an absolute position on the tablet",
            got(&out, &tablet, EV_ABS, ABS_X, |v| v == 16384) && got(&out, &tablet, EV_ABS, ABS_Y, |v| v == 8192),
        ),
        ("A on a keyboard", keyboards.iter().any(|d| got(&out, d, EV_KEY, KEY_A, |v| v == 1))),
        ("motion on a mouse", mice.iter().any(|d| got(&out, d, EV_REL, REL_X, |v| v > 0))),
    ])?;
    for (name, dev) in [("keyboard", &vkbd), ("mouse", &vmouse), ("tablet", &tablet)] {
        let n = out.lines().filter(|l| l.contains(&format!("evtest: {dev} type "))).count();
        eprintln!("==> virtio {name} ({dev}): {n} event(s)");
    }

    eprintln!("==> boot test: the kernel tests of the event layer with the three virtio devices");
    let append = format!("poly.qemu_exit poly.test=input {}=3", VIRTIO_TEST_FLAG);
    let qemu_args: Vec<String> = devices.iter().flat_map(|d| ["-device".to_string(), d.to_string()]).collect();
    let b = boot_with(kernel, syms, super::DEFAULT_MEMORY, &append, Devices::None, &qemu_args, None, |_| {})?;
    let report = b.tests(&b.output)?;
    let found = b.output.contains("test virtio_input::devices_found ... ok");
    if b.status != Some(QEMU_SUCCESS) || !report.failed.is_empty() || !found {
        return Err(format!("the event layer's kernel tests with virtio devices failed (QEMU status {:?})", b.status));
    }
    Ok(())
}

/// Must match `virtio_input::TEST_FLAG` in the kernel.
const VIRTIO_TEST_FLAG: &str = "poly.test.virtio_input";
