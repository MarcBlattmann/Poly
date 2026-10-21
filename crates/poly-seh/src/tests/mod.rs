//! Host tests: images built in memory (hand-assembled code, unwind info,
//! a function table) and stacks laid out the way the code would leave
//! them.

// Contexts are built field by field from a default one, as the code
// under test fills them.
#![allow(clippy::field_reassign_with_default)]

extern crate std;

mod dispatch;
mod fuzz;
mod info;
mod layout;
mod pdata;
mod unwind;

use crate::consts::*;
use crate::dispatch::{Module, Modules, StackLimits};
use crate::info::{self as uinfo, UnwindEntry, UnwindOp};
use crate::pdata::RuntimeFunction;
use crate::{BadAddress, Memory};
use alloc::vec;
use alloc::vec::Vec;

/// Where test images go.
pub const BASE: u64 = 0x1_4000_0000;
pub const CODE_RVA: u32 = 0x1000;
pub const XDATA_RVA: u32 = 0x1_0000;
pub const PDATA_RVA: u32 = 0x2_0000;
pub const IMAGE_SIZE: u64 = 0x3_0000;

/// The test stack.
pub const STACK_LOW: u64 = 0x7000_0000;
pub const STACK_HIGH: u64 = 0x7004_0000;
pub const LIMITS: StackLimits = StackLimits { low: STACK_LOW, high: STACK_HIGH };

/// Memory as a few byte regions.
#[derive(Clone, Debug, Default)]
pub struct Mem {
    regions: Vec<(u64, Vec<u8>)>,
}

impl Mem {
    pub fn map(&mut self, base: u64, len: usize) {
        self.regions.push((base, vec![0u8; len]));
    }

    fn region_mut(&mut self, addr: u64, len: usize) -> &mut [u8] {
        for (base, bytes) in &mut self.regions {
            if addr >= *base && addr - *base + len as u64 <= bytes.len() as u64 {
                let at = (addr - *base) as usize;
                return &mut bytes[at..at + len];
            }
        }
        panic!("test memory not mapped at {addr:#x}");
    }

    pub fn write(&mut self, addr: u64, data: &[u8]) {
        self.region_mut(addr, data.len()).copy_from_slice(data);
    }

    pub fn put64(&mut self, addr: u64, v: u64) {
        self.write(addr, &v.to_le_bytes());
    }

    pub fn put128(&mut self, addr: u64, v: u128) {
        self.write(addr, &v.to_le_bytes());
    }

    /// Memory with the test stack mapped.
    pub fn with_stack() -> Mem {
        let mut m = Mem::default();
        m.map(STACK_LOW, (STACK_HIGH - STACK_LOW) as usize);
        m
    }
}

impl Memory for Mem {
    fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), BadAddress> {
        for (base, bytes) in &self.regions {
            if addr >= *base && (addr - *base).checked_add(buf.len() as u64).is_some_and(|e| e <= bytes.len() as u64) {
                let at = (addr - *base) as usize;
                buf.copy_from_slice(&bytes[at..at + buf.len()]);
                return Ok(());
            }
        }
        Err(BadAddress(addr))
    }
}

/// An image under construction: code at [`CODE_RVA`], unwind info at
/// [`XDATA_RVA`], the function table at [`PDATA_RVA`].
#[derive(Clone, Debug, Default)]
pub struct Img {
    pub code: Vec<u8>,
    pub xdata: Vec<u8>,
    pub funcs: Vec<RuntimeFunction>,
}

/// A function's unwind description, for [`Img::func`].
#[derive(Clone, Debug, Default)]
pub struct Desc {
    pub flags: u8,
    pub prolog: u8,
    pub frame_reg: u8,
    pub frame_off: u8,
    pub ops: Vec<UnwindEntry>,
    /// Raw code slots instead of `ops`, if set.
    pub raw_codes: Option<Vec<u16>>,
    /// Handler RVA, or the chained function.
    pub handler: Option<u32>,
    pub handler_data: Vec<u8>,
    pub chain: Option<RuntimeFunction>,
}

pub fn e(prolog_offset: u8, op: UnwindOp) -> UnwindEntry {
    UnwindEntry { prolog_offset, op }
}

impl Img {
    /// Append unwind info bytes; returns their RVA.
    pub fn add_info(&mut self, bytes: &[u8]) -> u32 {
        while !self.xdata.len().is_multiple_of(4) {
            self.xdata.push(0);
        }
        let rva = XDATA_RVA + self.xdata.len() as u32;
        self.xdata.extend_from_slice(bytes);
        rva
    }

    /// Append code (16-byte aligned); returns its RVA.
    pub fn add_code(&mut self, code: &[u8]) -> u32 {
        while !self.code.len().is_multiple_of(16) {
            self.code.push(0xCC);
        }
        let rva = CODE_RVA + self.code.len() as u32;
        self.code.extend_from_slice(code);
        rva
    }

    pub fn info_bytes(d: &Desc) -> Vec<u8> {
        let codes = d.raw_codes.clone().unwrap_or_else(|| uinfo::encode(&d.ops));
        let mut trailer = Vec::new();
        if let Some(c) = d.chain {
            trailer.extend_from_slice(&c.to_bytes());
        } else if let Some(h) = d.handler {
            trailer.extend_from_slice(&h.to_le_bytes());
            trailer.extend_from_slice(&d.handler_data);
        }
        uinfo::build(d.flags, d.prolog, d.frame_reg, d.frame_off, &codes, &trailer)
    }

    /// A function: its code and unwind description. Returns its entry.
    pub fn func(&mut self, code: &[u8], d: &Desc) -> RuntimeFunction {
        let begin = self.add_code(code);
        let unwind = self.add_info(&Img::info_bytes(d));
        let f = RuntimeFunction { begin, end: begin + code.len() as u32, unwind };
        self.funcs.push(f);
        f
    }

    /// Map the image into `mem`; returns its module (table sorted).
    pub fn install(&self, mem: &mut Mem) -> Module {
        mem.map(BASE, IMAGE_SIZE as usize);
        mem.write(BASE + u64::from(CODE_RVA), &self.code);
        mem.write(BASE + u64::from(XDATA_RVA), &self.xdata);
        let mut funcs = self.funcs.clone();
        funcs.sort_by_key(|f| f.begin);
        let table: Vec<u8> = funcs.iter().flat_map(|f| f.to_bytes()).collect();
        mem.write(BASE + u64::from(PDATA_RVA), &table);
        Module::from_directory(BASE, IMAGE_SIZE, PDATA_RVA, table.len() as u32)
    }

    /// The image installed in a memory with the stack, as a target.
    pub fn target(&self, mut mem: Mem) -> Modules<Mem> {
        let m = self.install(&mut mem);
        Modules { memory: mem, modules: vec![m], stack: LIMITS }
    }
}

/// `push rbx; push rsi; sub rsp, 0x28; nop; nop; add rsp, 0x28; pop rsi;
/// pop rbx; ret` and its unwind description: the body is at offset 6, the
/// epilog at 8.
pub fn simple_function() -> (Vec<u8>, Desc) {
    let code = vec![
        0x53, // push rbx           (ends at 1)
        0x56, // push rsi           (ends at 2)
        0x48, 0x83, 0xEC, 0x28, // sub rsp, 0x28  (ends at 6)
        0x90, 0x90, // body
        0x48, 0x83, 0xC4, 0x28, // add rsp, 0x28 (8)
        0x5E, // pop rsi (12)
        0x5B, // pop rbx (13)
        0xC3, // ret (14)
    ];
    let d = Desc {
        prolog: 6,
        ops: vec![
            e(6, UnwindOp::Alloc { size: 0x28 }),
            e(2, UnwindOp::PushNonvol { reg: RSI }),
            e(1, UnwindOp::PushNonvol { reg: RBX }),
        ],
        ..Desc::default()
    };
    (code, d)
}

/// A deterministic pseudo-random generator for the fuzz tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}
