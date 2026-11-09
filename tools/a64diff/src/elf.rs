//! A minimal static `aarch64` ELF64 loader and a system-call shim that runs
//! a program on [`poly_a64::Cpu`]. Host tool only.
//!
//! The shim implements just what the test programs use: `write` (64),
//! `exit` (93) and `exit_group` (94). Every other system call returns
//! `-ENOSYS`.

use poly_a64::{Access, Cpu, MemFault, Memory, Outcome};

/// One loaded range of the guest address space.
struct Region {
    base: u64,
    data: Vec<u8>,
}

/// Guest memory made of a few separate regions.
pub struct Mem {
    regions: Vec<Region>,
}

impl Mem {
    fn find(&self, addr: u64, len: usize, access: Access) -> Result<(usize, usize), MemFault> {
        for (i, r) in self.regions.iter().enumerate() {
            if addr >= r.base && addr - r.base < r.data.len() as u64 {
                let off = (addr - r.base) as usize;
                if off + len <= r.data.len() {
                    return Ok((i, off));
                }
                return Err(MemFault { addr: r.base + r.data.len() as u64, access, alignment: false });
            }
        }
        Err(MemFault { addr, access, alignment: false })
    }
}

impl Memory for Mem {
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), MemFault> {
        let (i, off) = self.find(addr, buf.len(), Access::Read)?;
        buf.copy_from_slice(&self.regions[i].data[off..off + buf.len()]);
        Ok(())
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), MemFault> {
        let (i, off) = self.find(addr, data.len(), Access::Write)?;
        self.regions[i].data[off..off + data.len()].copy_from_slice(data);
        Ok(())
    }
}

fn u16_at(b: &[u8], o: usize) -> Result<u16, String> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "ELF truncated".to_string())
}
fn u32_at(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "ELF truncated".to_string())
}
fn u64_at(b: &[u8], o: usize) -> Result<u64, String> {
    b.get(o..o + 8)
        .map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
        .ok_or_else(|| "ELF truncated".to_string())
}

/// What a program did.
#[derive(Debug)]
pub struct RunResult {
    /// Bytes written to file descriptor 1.
    pub stdout: Vec<u8>,
    /// How it ended.
    pub end: End,
    /// Instructions executed.
    pub steps: u64,
}

/// How a run ended.
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    /// `exit(code)`.
    Exit(u64),
    /// The interpreter reported something it cannot continue from.
    Stopped(String),
}

const STACK_TOP: u64 = 0x7fff_f000_0000;
const STACK_SIZE: u64 = 1 << 20;

/// Load a static `aarch64` ELF executable.
pub fn load(elf: &[u8]) -> Result<(Mem, u64), String> {
    if elf.get(0..4) != Some(b"\x7fELF") || elf.get(4) != Some(&2) || elf.get(5) != Some(&1) {
        return Err("not a little-endian ELF64 file".into());
    }
    if u16_at(elf, 18)? != 183 {
        return Err("not an aarch64 ELF".into());
    }
    let entry = u64_at(elf, 24)?;
    let phoff = u64_at(elf, 32)? as usize;
    let phentsize = u16_at(elf, 54)? as usize;
    let phnum = u16_at(elf, 56)? as usize;
    let mut regions = Vec::new();
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if u32_at(elf, ph)? != 1 {
            continue; // not PT_LOAD
        }
        let offset = u64_at(elf, ph + 8)? as usize;
        let vaddr = u64_at(elf, ph + 16)?;
        let filesz = u64_at(elf, ph + 32)? as usize;
        let memsz = u64_at(elf, ph + 40)? as usize;
        let mut data = vec![0u8; memsz];
        let src = elf.get(offset..offset + filesz).ok_or("segment outside the file")?;
        data[..filesz].copy_from_slice(src);
        regions.push(Region { base: vaddr, data });
    }
    regions.push(Region { base: STACK_TOP - STACK_SIZE, data: vec![0; STACK_SIZE as usize] });
    Ok((Mem { regions }, entry))
}

/// The symbols of the ELF's `.symtab` as `(address, name)`.
pub fn symbols(elf: &[u8]) -> Result<Vec<(u64, String)>, String> {
    let shoff = u64_at(elf, 40)? as usize;
    let shentsize = u16_at(elf, 58)? as usize;
    let shnum = u16_at(elf, 60)? as usize;
    let mut out = Vec::new();
    for i in 0..shnum {
        let sh = shoff + i * shentsize;
        if u32_at(elf, sh + 4)? != 2 {
            continue; // not SHT_SYMTAB
        }
        let off = u64_at(elf, sh + 24)? as usize;
        let size = u64_at(elf, sh + 32)? as usize;
        let link = u32_at(elf, sh + 40)? as usize;
        let strsh = shoff + link * shentsize;
        let stroff = u64_at(elf, strsh + 24)? as usize;
        for k in 0..size / 24 {
            let e = off + k * 24;
            let name_off = u32_at(elf, e)? as usize;
            let value = u64_at(elf, e + 8)?;
            let start = stroff + name_off;
            let end = elf[start..].iter().position(|&b| b == 0).map(|p| start + p).ok_or("bad string table")?;
            out.push((value, String::from_utf8_lossy(&elf[start..end]).into_owned()));
        }
    }
    Ok(out)
}

/// Run `elf` on poly-a64 for at most `max_steps` instructions.
pub fn run(elf: &[u8], max_steps: u64, dczid: u64) -> Result<RunResult, String> {
    let (mut mem, entry) = load(elf)?;
    let mut cpu = Cpu::new();
    cpu.dczid = dczid;
    cpu.pc = entry;
    // argc = 0, argv = NULL, envp = NULL, auxv = AT_NULL: all zero already.
    cpu.sp = STACK_TOP - 64;
    let mut stdout = Vec::new();
    let mut steps = 0u64;
    let end = loop {
        if steps >= max_steps {
            break End::Stopped(format!("step limit {max_steps} reached at pc {:#x}", cpu.pc));
        }
        steps += 1;
        match cpu.step(&mut mem) {
            Outcome::Continue => {}
            Outcome::Svc(_) => match cpu.x[8] {
                64 => {
                    let (fd, buf, len) = (cpu.x[0], cpu.x[1], cpu.x[2] as usize);
                    let mut data = vec![0u8; len];
                    match mem.read(buf, &mut data) {
                        Ok(()) => {
                            if fd == 1 {
                                stdout.extend_from_slice(&data);
                            }
                            cpu.x[0] = len as u64;
                        }
                        Err(_) => cpu.x[0] = (-14i64) as u64, // EFAULT
                    }
                }
                93 | 94 => break End::Exit(cpu.x[0]),
                _ => cpu.x[0] = (-38i64) as u64, // ENOSYS
            },
            other => break End::Stopped(format!("{other:?} at pc {:#x}", cpu.pc)),
        }
    };
    Ok(RunResult { stdout, end, steps })
}

/// A block that poly-a64 could not execute.
#[derive(Debug)]
pub struct BlockStop {
    /// The block number (the `N` of the `blk_N` label).
    pub block: usize,
    /// What stopped it.
    pub why: String,
}

/// Run a generated fuzz program block by block: when poly-a64 stops inside
/// a block (undefined or unsupported instruction, fault, step limit), note
/// it and resume at the next block's label, so one gap does not hide the
/// rest. Returns the output and the stopped blocks.
pub fn run_blocks(elf: &[u8], steps_per_block: u64, dczid: u64) -> Result<(Vec<u8>, Vec<BlockStop>), String> {
    let syms = symbols(elf)?;
    let mut blocks: Vec<(u64, usize)> = Vec::new(); // (address, number)
    let mut end_addr = 0;
    for (addr, name) in &syms {
        if let Some(n) = name.strip_prefix("blk_") {
            if n == "end" {
                end_addr = *addr;
            } else if let Ok(i) = n.parse::<usize>() {
                blocks.push((*addr, i));
            }
        }
    }
    blocks.sort();
    if end_addr == 0 {
        return Err("program has no blk_end label".into());
    }
    let (mut mem, entry) = load(elf)?;
    let mut cpu = Cpu::new();
    cpu.dczid = dczid;
    cpu.pc = entry;
    cpu.sp = STACK_TOP - 64;
    let mut stdout = Vec::new();
    let mut stops = Vec::new();
    let mut block_steps = 0u64;
    let mut cur_block_start = entry;
    loop {
        block_steps += 1;
        let pc = cpu.pc;
        if blocks.binary_search_by_key(&pc, |b| b.0).is_ok() {
            cur_block_start = pc;
            block_steps = 0;
        }
        let stop: Option<String> = if block_steps > steps_per_block {
            Some(format!("step limit at pc {pc:#x}"))
        } else {
            match cpu.step(&mut mem) {
                Outcome::Continue => None,
                Outcome::Svc(_) => match cpu.x[8] {
                    64 => {
                        let (fd, buf, len) = (cpu.x[0], cpu.x[1], cpu.x[2] as usize);
                        let mut data = vec![0u8; len];
                        if mem.read(buf, &mut data).is_ok() && fd == 1 {
                            stdout.extend_from_slice(&data);
                        }
                        cpu.x[0] = len as u64;
                        None
                    }
                    93 | 94 => return Ok((stdout, stops)),
                    _ => {
                        cpu.x[0] = (-38i64) as u64;
                        None
                    }
                },
                other => Some(format!("{other:?} at pc {pc:#x}")),
            }
        };
        if let Some(why) = stop {
            let idx = blocks.partition_point(|b| b.0 <= cur_block_start).saturating_sub(1);
            let Some(&(_, number)) = blocks.get(idx) else { return Err(format!("stopped outside a block: {why}")) };
            stops.push(BlockStop { block: number, why });
            cpu.pc = blocks.get(idx + 1).map(|b| b.0).unwrap_or(end_addr);
            cur_block_start = cpu.pc;
            block_steps = 0;
        }
    }
}
