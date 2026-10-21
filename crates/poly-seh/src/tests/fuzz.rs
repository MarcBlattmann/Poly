//! Random and truncated input never makes the crate panic.

use super::{BASE, Desc, Img, LIMITS, Mem, Rng, STACK_HIGH, STACK_LOW, simple_function};
use crate::context::Context;
use crate::dispatch::{DispatchInfo, Disposition, Handlers, Module, Modules, dispatch_exception, unwind, walk};
use crate::info::{UnwindInfo, decode};
use crate::pdata::{FunctionTable, RuntimeFunction};
use crate::record::ExceptionRecord;
use crate::unwind::virtual_unwind;
use crate::{SliceMemory, consts::*};
use alloc::vec;

struct Search;

impl Handlers for Search {
    fn call(&mut self, _: &mut ExceptionRecord, _: u64, _: &mut Context, _: &mut DispatchInfo) -> Disposition {
        Disposition::ContinueSearch
    }
}

fn random_context(rng: &mut Rng, code_begin: u64, code_len: u64) -> Context {
    let mut c = Context::default();
    for r in c.gpr.iter_mut() {
        // Mostly stack-ish values, sometimes anything.
        *r = if rng.below(4) == 0 { rng.next() } else { STACK_LOW + rng.below(STACK_HIGH - STACK_LOW) };
    }
    c.set_rsp(STACK_LOW + rng.below(STACK_HIGH - STACK_LOW + 0x100));
    c.rip = code_begin + rng.below(code_len.max(1) + 4);
    c
}

#[test]
fn random_unwind_info_code_and_stacks() {
    let mut rng = Rng(0xDEAD_BEEF_1234_5678);
    for round in 0..3000 {
        // Random unwind info bytes (a valid header half the time), random
        // code and a random stack.
        let n = 4 + rng.below(80) as usize;
        let mut info = rng.bytes(n);
        if round % 2 == 0 {
            info[0] = 1 | ((rng.next() as u8 & 7) << 3);
        }
        let n = 1 + rng.below(64) as usize;
        let code = rng.bytes(n);
        let mut img = Img::default();
        let begin = img.add_code(&code);
        let info_rva = img.add_info(&info);
        let f = RuntimeFunction { begin, end: begin + code.len() as u32, unwind: info_rva };
        img.funcs.push(f);
        // Sometimes a second, chained-to entry.
        if rng.below(3) == 0 {
            let n = 4 + rng.below(40) as usize;
            let mut more = rng.bytes(n);
            more[0] = 1 | (4 << 3);
            let u2 = img.add_info(&more);
            img.funcs.push(RuntimeFunction { begin: begin + 0x400, end: begin + 0x410, unwind: u2 });
        }
        let mut mem = Mem::with_stack();
        let m = img.install(&mut mem);
        for _ in 0..8 {
            let at = STACK_LOW + rng.below(STACK_HIGH - STACK_LOW - 8);
            mem.put64(at, rng.next());
        }
        let mut c = random_context(&mut rng, BASE + u64::from(begin), code.len() as u64);
        let control = c.rip;
        for kind in [UNW_FLAG_NHANDLER, UNW_FLAG_EHANDLER, UNW_FLAG_UHANDLER] {
            let mut cc = c.clone();
            let _ = virtual_unwind(kind, BASE, control, &f, &mut cc, &mem, None);
        }
        let target = Modules { memory: mem, modules: vec![m], stack: LIMITS };
        let mut rec = ExceptionRecord::new(STATUS_ACCESS_VIOLATION, 0, c.rip, &[]);
        let _ = dispatch_exception(&target, &mut Search, &mut rec, &mut c);
        let _ = unwind(&target, &mut Search, Some(rng.next()), 0, &mut rec, 0, &c);
        let _ = walk(&target, &c, 64);
    }
}

#[test]
fn random_code_arrays_and_headers() {
    let mut rng = Rng(42);
    for _ in 0..20_000 {
        let n = rng.below(40) as usize;
        let codes: alloc::vec::Vec<u16> = (0..n).map(|_| rng.next() as u16).collect();
        let _ = decode(&codes);
        let n = rng.below(600) as usize;
        let bytes = rng.bytes(n);
        let mem = SliceMemory { base: 0x1000, bytes: &bytes };
        if let Ok(info) = UnwindInfo::read(&mem, 0x1000) {
            let _ = info.ops();
            let _ = (info.handler_data(), info.size(), info.tail());
        }
        let _ = ExceptionRecord::from_bytes(&bytes);
        let _ = Context::from_bytes(&bytes);
        if let Ok(t) = FunctionTable::new(&bytes[..bytes.len() / 12 * 12]) {
            let _ = t.validate();
            let _ = t.lookup(rng.next() as u32);
        }
    }
}

#[test]
fn every_truncation_of_a_valid_function_is_handled() {
    let (code, d) = simple_function();
    let full = Img::info_bytes(&Desc { flags: UNW_FLAG_EHANDLER, handler: Some(0x5000), ..d });
    for cut in 0..full.len() {
        let mut img = Img::default();
        let begin = img.add_code(&code);
        let info_rva = img.add_info(&full[..cut]);
        let f = RuntimeFunction { begin, end: begin + code.len() as u32, unwind: info_rva };
        // Only the cut bytes are mapped: the image region is the bytes.
        let mut mem = Mem::with_stack();
        mem.map(BASE + u64::from(begin), code.len());
        mem.write(BASE + u64::from(begin), &code);
        mem.map(BASE + u64::from(info_rva), cut.max(1));
        mem.write(BASE + u64::from(info_rva), &full[..cut]);
        let mut c = Context::default();
        c.rip = BASE + u64::from(begin) + 6;
        c.set_rsp(STACK_HIGH - 0x100);
        let r = virtual_unwind(UNW_FLAG_EHANDLER, BASE, c.rip, &f, &mut c, &mem, None);
        assert!(r.is_err(), "cut at {cut} of {}", full.len());
    }
    // Module edge cases: a table that points outside memory.
    let m = Module { base: BASE, size: 0x1000, table: u64::MAX - 8, count: 1000 };
    let t = Modules { memory: Mem::default(), modules: vec![m], stack: LIMITS };
    let mut c = Context::default();
    c.rip = BASE + 0x10;
    c.set_rsp(STACK_HIGH - 8);
    let _ = walk(&t, &c, 8);
}
