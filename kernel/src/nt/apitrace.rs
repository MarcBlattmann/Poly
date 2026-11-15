//! `poly.nt.apitrace=<text>`: a diagnostic for a 32-bit program that fails inside its own code,
//! where the system calls say nothing (ADR 0186, stage 4). From the first system call whose object
//! name contains `<text>` (case-insensitive: `poly.nt.apitrace=msctf` starts at the program's probe
//! for `Msctf.dll`), the calling thread single-steps (RFLAGS.TF) and the kernel logs every entry
//! into an export of a module of the process other than the program: `nt-api: kernel32.dll!Name
//! ret=... args ...`. It reads Poly's own export tables and the thread's stack, never the
//! program's code, so it is an observation of what the program calls and in which order.
//! A trace is slow (a trap per instruction), so it is limited to [`LIMIT`] lines.

use super::{Mutex, String, Vec};
use crate::arch::idt::TrapFrame;
use crate::user;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_abi::SyscallRegs;

/// The lines one trace logs at most.
const LIMIT: u32 = 400_000;
/// RFLAGS.TF.
const TF: u64 = 1 << 8;

/// `poly.nt.apitrace` was given.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The thread being traced (`pid << 32 | tid`), 0 when none.
static THREAD: AtomicU64 = AtomicU64::new(0);
static NEEDLE: Mutex<String> = Mutex::new(String::new());
static EXPORTS: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());
static LEFT: AtomicU64 = AtomicU64::new(0);
/// The last instructions the traced thread ran: (eip, eax, ecx, edx, ebx, esi, edi), newest last.
static RING: Mutex<Vec<[u32; 7]>> = Mutex::new(Vec::new());
/// How many instructions the ring keeps.
const RING_LEN: usize = 64;
/// Steps since the call named by `poly.nt.apitrace.after` returned (0: not started).
static SAMPLING: AtomicU64 = AtomicU64::new(0);
static AFTER: Mutex<String> = Mutex::new(String::new());
/// The calls entered and not yet returned: (return address, stack pointer at entry, name).
static PENDING: Mutex<Vec<(u32, u32, String)>> = Mutex::new(Vec::new());

/// Read the kernel argument.
pub fn configure(boot: &crate::boot_info::BootInfo) {
    if let Some(text) = boot.arg("poly.nt.apitrace.after") {
        *AFTER.lock() = text.to_ascii_lowercase();
    }
    if let Some(text) = boot.arg("poly.nt.apitrace") {
        *NEEDLE.lock() = text.to_ascii_lowercase();
        ACTIVE.store(true, Ordering::Relaxed);
        crate::kprintln!(
            "nt: tracing the calls into the system DLLs after a system call naming {text:?} (poly.nt.apitrace)"
        );
    }
}

/// Whether the diagnostic is on and has not started (the syscall path then reads each call's object name).
pub fn wanted() -> bool {
    ACTIVE.load(Ordering::Relaxed) && THREAD.load(Ordering::Relaxed) == 0
}

fn u32_at(at: u64) -> Option<u32> {
    let mut b = [0u8; 4];
    user::copy_from_user(&mut b, at).ok()?;
    Some(u32::from_le_bytes(b))
}

/// The exports (address, `module!name`) of one 32-bit module mapped at `base`.
fn exports_of(name: &str, base: u64, out: &mut Vec<(u32, String)>) {
    let Some(nt) = u32_at(base + 0x3c).map(u64::from) else { return };
    // PE32: the export directory is the first data directory, 96 bytes into the optional header.
    let (Some(rva), Some(size)) = (u32_at(base + nt + 24 + 96), u32_at(base + nt + 24 + 100)) else { return };
    if rva == 0 {
        return;
    }
    let dir = base + u64::from(rva);
    let (Some(count), Some(names), Some(functions), Some(name_table), Some(ordinals)) =
        (u32_at(dir + 0x14), u32_at(dir + 0x18), u32_at(dir + 0x1c), u32_at(dir + 0x20), u32_at(dir + 0x24))
    else {
        return;
    };
    let module = name.to_ascii_lowercase();
    for i in 0..u64::from(names.min(20_000)) {
        let Some(name_rva) = u32_at(base + u64::from(name_table) + 4 * i) else { continue };
        let mut ordinal = [0u8; 2];
        if user::copy_from_user(&mut ordinal, base + u64::from(ordinals) + 2 * i).is_err() {
            continue;
        }
        let index = u64::from(u16::from_le_bytes(ordinal));
        if index >= u64::from(count) {
            continue;
        }
        let Some(function) = u32_at(base + u64::from(functions) + 4 * index) else { continue };
        // A forwarder's "address" is a string inside the export directory.
        if function >= rva && function < rva.saturating_add(size) {
            continue;
        }
        let mut text = [0u8; 80];
        let _ = user::copy_from_user(&mut text, base + u64::from(name_rva));
        let len = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        out.push((
            (base as u32).wrapping_add(function),
            alloc::format!("{module}!{}", String::from_utf8_lossy(&text[..len])),
        ));
    }
}

/// The modules of a 32-bit process on `PEB.Ldr`'s load-order list, as (base name, base): `PEB` is at
/// `TEB + 0x30`, `Ldr` at `PEB + 0xC`, the list head at `Ldr + 0xC`, and an entry has `DllBase` at
/// +0x18 and `BaseDllName` (a 32-bit `UNICODE_STRING`) at +0x2C.
fn modules32(teb: u64) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let Some(peb) = u32_at(teb + 0x30) else { return out };
    let Some(ldr) = u32_at(u64::from(peb) + 0xC) else { return out };
    let head = u64::from(ldr) + 0xC;
    let mut at = u32_at(head).map_or(0, u64::from);
    while at != head && at != 0 && out.len() < 256 {
        if let Some(base) = u32_at(at + 0x18).filter(|&b| b != 0) {
            let length = u32_at(at + 0x2C).unwrap_or(0) & 0xffff;
            let buffer = u32_at(at + 0x30).unwrap_or(0);
            let mut bytes = alloc::vec![0u8; (length as usize).min(256)];
            let _ = user::copy_from_user(&mut bytes, u64::from(buffer));
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            out.push((String::from_utf16_lossy(&units), u64::from(base)));
        }
        at = u32_at(at).map_or(0, u64::from);
    }
    out
}

/// Called after each traced system call of a 32-bit thread with the object name it named, if any:
/// starts the trace when it matches.
pub fn arm_if_match(pid: u32, tid: u32, object: Option<&str>, teb: u64, regs: &mut SyscallRegs) {
    if !wanted() {
        return;
    }
    let Some(object) = object else { return };
    if !object.to_ascii_lowercase().contains(NEEDLE.lock().as_str()) {
        return;
    }
    let modules = modules32(teb);
    let mut table = Vec::new();
    for (name, base) in modules.iter().skip(1) {
        exports_of(name, *base, &mut table);
    }
    table.sort_by_key(|e| e.0);
    table.dedup_by_key(|e| e.0);
    crate::kprintln!("nt-api: p{pid} t{tid} traces from here: {} exports of {} modules", table.len(), modules.len());
    *EXPORTS.lock() = table;
    LEFT.store(u64::from(LIMIT), Ordering::Relaxed);
    THREAD.store(u64::from(pid) << 32 | u64::from(tid), Ordering::Relaxed);
    regs.rflags |= TF;
}

/// A single-step trap from ring 3 (vector 1): logs an entry into an export, keeps stepping. False
/// when the trap is not the traced thread's (then it is an ordinary exception).
pub fn step(frame: &mut TrapFrame) -> bool {
    let thread = THREAD.load(Ordering::Relaxed);
    if thread == 0 || user::current_tid().map(u64::from) != Some(thread & 0xffff_ffff) {
        return false;
    }
    frame.rflags |= TF;
    let at = frame.rip as u32;
    {
        let mut ring = RING.lock();
        if ring.len() == RING_LEN {
            ring.remove(0);
        }
        ring.push([
            at,
            frame.rax as u32,
            frame.rcx as u32,
            frame.rdx as u32,
            frame.rbx as u32,
            frame.rsi as u32,
            frame.rdi as u32,
        ]);
    }
    let sampling = SAMPLING.load(Ordering::Relaxed);
    if sampling != 0 {
        SAMPLING.store(sampling + 1, Ordering::Relaxed);
        if sampling < 400 || sampling % 2_000_000 == 0 {
            crate::kprintln!(
                "nt-api: step {sampling}: eip={at:#x} eax={:#x} ecx={:#x} edx={:#x} ebx={:#x} esi={:#x} edi={:#x} esp={:#x}",
                frame.rax as u32,
                frame.rcx as u32,
                frame.rdx as u32,
                frame.rbx as u32,
                frame.rsi as u32,
                frame.rdi as u32,
                frame.rsp as u32
            );
        }
        if sampling == 185 || sampling == 197 {
            let mut b = [0u8; 96];
            if user::copy_from_user(&mut b, frame.rax & 0xffff_ffff).is_ok() {
                let hex: String = b.iter().map(|x| alloc::format!("{x:02x}")).collect();
                crate::kprintln!("nt-api: memory at eax at step {sampling}: {hex}");
            }
            let mut b = [0u8; 96];
            if user::copy_from_user(&mut b, frame.rdx & 0xffff_ffff).is_ok() {
                let hex: String = b.iter().map(|x| alloc::format!("{x:02x}")).collect();
                crate::kprintln!("nt-api: memory at edx at step {sampling}: {hex}");
            }
        }
        // Only the steps after the call matter: no more call lines.
        return true;
    }
    // A call that returns: its result.
    {
        let mut pending = PENDING.lock();
        while let Some((ret, esp, name)) = pending.last().cloned() {
            if at == ret && (frame.rsp as u32) > esp {
                pending.pop();
                crate::kprintln!("nt-api:   {name} -> {:#x}", frame.rax as u32);
                let after = AFTER.lock();
                if !after.is_empty() && name.to_ascii_lowercase().contains(after.as_str()) {
                    SAMPLING.store(1, Ordering::Relaxed);
                }
            } else if (frame.rsp as u32) > esp.wrapping_add(0x400) && at != ret {
                // Returned some other way (an exception): forget it.
                pending.pop();
            } else {
                break;
            }
        }
    }
    let found = {
        let table = EXPORTS.lock();
        table.binary_search_by_key(&at, |e| e.0).ok().map(|i| table[i].1.clone())
    };
    if let Some(name) = found {
        if LEFT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1)).is_ok() {
            let mut words = [0u8; 20];
            let _ = user::copy_from_user(&mut words, frame.rsp);
            let w = |i: usize| u32::from_le_bytes([words[4 * i], words[4 * i + 1], words[4 * i + 2], words[4 * i + 3]]);
            crate::kprintln!("nt-api: {name} ret={:#x} args {:#x} {:#x} {:#x} {:#x}", w(0), w(1), w(2), w(3), w(4));
            if name.contains("InitializeCriticalSection") {
                // The object around a critical section: what the constructor wrote before it.
                let mut b = [0u8; 64];
                if user::copy_from_user(&mut b, u64::from(w(1).wrapping_sub(16))).is_ok() {
                    let hex: String = b.iter().map(|x| alloc::format!("{x:02x}")).collect();
                    crate::kprintln!("nt-api:   memory at arg0-16: {hex}");
                }
            }
            let mut pending = PENDING.lock();
            if pending.len() < 64 {
                pending.push((w(0), frame.rsp as u32, name));
            }
        } else {
            // The limit is reached: stop stepping.
            THREAD.store(0, Ordering::Relaxed);
            frame.rflags &= !TF;
            crate::kprintln!("nt-api: stopped after {LIMIT} calls");
        }
    }
    true
}

/// An exception reached the traced thread: log the instructions it ran last, with the registers
/// before each (the report of `poly.nt.exceptions` has the registers at the fault).
pub fn dump_ring() {
    if THREAD.load(Ordering::Relaxed) == 0 {
        return;
    }
    for r in RING.lock().iter() {
        crate::kprintln!(
            "nt-api: last steps: eip={:#x} eax={:#x} ecx={:#x} edx={:#x} ebx={:#x} esi={:#x} edi={:#x}",
            r[0],
            r[1],
            r[2],
            r[3],
            r[4],
            r[5],
            r[6]
        );
    }
}
