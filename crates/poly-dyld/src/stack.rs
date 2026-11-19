//! The initial stack of a Mach-O program.
//!
//! From the stack pointer up: the address of the program's Mach-O header
//! (only when a dynamic linker runs first: it is how dyld finds the
//! program), `argc`, the `argv` pointers and a null, the `envp` pointers and
//! a null, the `apple` pointers and a null; the strings above them. The
//! `apple` strings are `NAME=value` facts for the system library; the first
//! is `executable_path=<path>`, the path the program was started by, which
//! `main`'s fourth argument shows.

use alloc::vec::Vec;

/// A built stack: `bytes` go at `sp` and end at the top given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    pub sp: u64,
    pub bytes: Vec<u8>,
}

/// Where each part went (for tests and diagnostics).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackLayout {
    pub argc_at: u64,
    pub argv: u64,
    pub envp: u64,
    pub apple: u64,
}

/// Build the stack that ends at `top` (16-byte aligned). `header`: the
/// program's Mach-O header address, pushed below `argc` when dyld runs.
/// `None` if the stack would wrap below address 0.
pub fn build(
    top: u64,
    argv: &[&[u8]],
    envp: &[&[u8]],
    apple: &[&[u8]],
    header: Option<u64>,
) -> Option<(Stack, StackLayout)> {
    let top = top & !15;
    let strings: usize = argv.iter().chain(envp).chain(apple).map(|s| s.len() + 1).sum();
    let strings_start = top.checked_sub(strings as u64)?;
    let words = header.is_some() as usize + 1 + argv.len() + 1 + envp.len() + 1 + apple.len() + 1;
    let sp = (strings_start & !15).checked_sub(words as u64 * 8)? & !15;
    let len = usize::try_from(top - sp).ok()?;
    let mut bytes = alloc::vec![0u8; len];
    // The strings, in order, from `strings_start` up.
    let mut ptrs = Vec::with_capacity(argv.len() + envp.len() + apple.len());
    let mut at = strings_start;
    for st in argv.iter().chain(envp).chain(apple) {
        let o = (at - sp) as usize;
        bytes[o..o + st.len()].copy_from_slice(st);
        ptrs.push(at);
        at += st.len() as u64 + 1;
    }
    let (pa, rest) = ptrs.split_at(argv.len());
    let (pe, pp) = rest.split_at(envp.len());
    let mut words: Vec<u64> = Vec::with_capacity(words);
    words.extend(header);
    words.push(argv.len() as u64);
    words.extend_from_slice(pa);
    words.push(0);
    words.extend_from_slice(pe);
    words.push(0);
    words.extend_from_slice(pp);
    words.push(0);
    for (i, w) in words.iter().enumerate() {
        bytes[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
    }
    let argc_at = sp + 8 * header.is_some() as u64;
    let argv_at = argc_at + 8;
    let envp_at = argv_at + 8 * (argv.len() as u64 + 1);
    let apple_at = envp_at + 8 * (envp.len() as u64 + 1);
    Some((Stack { sp, bytes }, StackLayout { argc_at, argv: argv_at, envp: envp_at, apple: apple_at }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(st: &Stack, at: u64) -> u64 {
        let o = (at - st.sp) as usize;
        u64::from_le_bytes(st.bytes[o..o + 8].try_into().unwrap())
    }

    fn string(st: &Stack, at: u64) -> &[u8] {
        let o = (at - st.sp) as usize;
        let end = st.bytes[o..].iter().position(|&b| b == 0).unwrap();
        &st.bytes[o..o + end]
    }

    #[test]
    fn dyld_stack_has_the_header_below_argc() {
        let top = 0x7fff_0000_1000;
        let (st, l) =
            build(top, &[b"/bin/x", b"a"], &[b"A=1"], &[b"executable_path=/bin/x"], Some(0x1_0000_0000)).unwrap();
        assert_eq!(st.sp % 16, 0);
        assert_eq!(st.sp + st.bytes.len() as u64, top);
        assert_eq!(word(&st, st.sp), 0x1_0000_0000);
        assert_eq!(l.argc_at, st.sp + 8);
        assert_eq!(word(&st, l.argc_at), 2);
        assert_eq!(string(&st, word(&st, l.argv)), b"/bin/x");
        assert_eq!(string(&st, word(&st, l.argv + 8)), b"a");
        assert_eq!(word(&st, l.argv + 16), 0);
        assert_eq!(l.envp, l.argv + 24);
        assert_eq!(string(&st, word(&st, l.envp)), b"A=1");
        assert_eq!(word(&st, l.envp + 8), 0);
        assert_eq!(l.apple, l.envp + 16);
        assert_eq!(string(&st, word(&st, l.apple)), b"executable_path=/bin/x");
        assert_eq!(word(&st, l.apple + 8), 0);
    }

    #[test]
    fn static_stack_starts_at_argc() {
        let (st, l) = build(0x10_0000, &[b"p"], &[], &[], None).unwrap();
        assert_eq!(l.argc_at, st.sp);
        assert_eq!(st.sp % 16, 0);
        assert_eq!(word(&st, st.sp), 1);
        assert_eq!(word(&st, l.envp), 0);
        assert_eq!(word(&st, l.apple), 0);
    }

    #[test]
    fn refuses_to_wrap() {
        assert!(build(16, &[b"long argument"], &[], &[], None).is_none());
    }
}
