//! Test helpers: synthetic tables with correct checksums, a byte-buffer
//! [`PhysMemory`], and walkers that touch every accessor of a parsed table.

extern crate std;

use crate::fadt::{
    CENTURY, DSDT, FADT_MINOR_VERSION, FLAGS, IAPC_BOOT_ARCH, PM_TMR_BLK, PM_TMR_LEN, RESET_REG, RESET_VALUE, SCI_INT,
    SMI_CMD, X_DSDT, X_PM_TMR_BLK,
};
use crate::*;
use std::hint::black_box;
use std::vec::Vec;

pub const OEM_ID: [u8; 6] = *b"POLYOS";
pub const FADT_FULL_LEN: usize = 276;
pub const DSDT_AML: &[u8] = &[0x10, 0x05, b'\\', b'_', b'S', b'B', b'_'];

/// Set the byte that makes the structure sum to zero.
fn fix_sum(b: &mut [u8], len: usize, at: usize) {
    if at >= b.len() {
        return;
    }
    b[at] = 0;
    let sum = b[..len.min(b.len())].iter().fold(0u8, |s, &x| s.wrapping_add(x));
    b[at] = 0u8.wrapping_sub(sum);
}

/// Recompute an SDT's checksum over its length field (or the buffer, if shorter).
pub fn fix_checksum(t: &mut [u8]) {
    let len = u32_at(t, 4).map_or(t.len(), |l| l as usize);
    fix_sum(t, len, 9);
}

/// Write `v` at `off` if it fits entirely.
pub fn put(t: &mut [u8], off: usize, v: &[u8]) {
    if let Some(dst) = t.get_mut(off..off + v.len()) {
        dst.copy_from_slice(v);
    }
}

pub fn set_length(t: &mut [u8], len: u32) {
    put(t, 4, &len.to_le_bytes());
    fix_checksum(t);
}

/// A complete table: header, `body`, valid checksum.
pub fn table(signature: &[u8; 4], revision: u8, body: &[u8]) -> Vec<u8> {
    let mut t = std::vec![0u8; SdtHeader::LEN];
    put(&mut t, 0, signature);
    put(&mut t, 4, &((SdtHeader::LEN + body.len()) as u32).to_le_bytes());
    t[8] = revision;
    put(&mut t, 10, &OEM_ID);
    put(&mut t, 16, b"POLYTEST");
    put(&mut t, 24, &1u32.to_le_bytes());
    put(&mut t, 28, b"PLY ");
    put(&mut t, 32, &1u32.to_le_bytes());
    t.extend_from_slice(body);
    fix_checksum(&mut t);
    t
}

fn rsdp_fix(r: &mut [u8]) {
    fix_sum(r, Rsdp::V1_LEN, 8);
    if r.len() >= Rsdp::V2_LEN {
        let len = u32_at(r, 20).map_or(r.len(), |l| l as usize);
        fix_sum(r, len, 32);
    }
}

pub fn rsdp_v1(rsdt: u32) -> Vec<u8> {
    let mut r = std::vec![0u8; Rsdp::V1_LEN];
    put(&mut r, 0, &Rsdp::SIGNATURE);
    put(&mut r, 9, &OEM_ID);
    put(&mut r, 16, &rsdt.to_le_bytes());
    rsdp_fix(&mut r);
    r
}

pub fn rsdp_v2(rsdt: u32, xsdt: u64) -> Vec<u8> {
    let mut r = rsdp_v1(rsdt);
    r.resize(Rsdp::V2_LEN, 0);
    r[15] = 2;
    put(&mut r, 20, &(Rsdp::V2_LEN as u32).to_le_bytes());
    put(&mut r, 24, &xsdt.to_le_bytes());
    rsdp_fix(&mut r);
    r
}

pub fn set_rsdp_length(r: &mut [u8], len: u32) {
    put(r, 20, &len.to_le_bytes());
    rsdp_fix(r);
}

// MADT entries (§5.2.12).

pub fn lapic(uid: u8, id: u8, flags: u32) -> Vec<u8> {
    let mut e = std::vec![0, 8, uid, id];
    e.extend_from_slice(&flags.to_le_bytes());
    e
}

pub fn ioapic(id: u8, address: u32, gsi_base: u32) -> Vec<u8> {
    let mut e = std::vec![1, 12, id, 0];
    e.extend_from_slice(&address.to_le_bytes());
    e.extend_from_slice(&gsi_base.to_le_bytes());
    e
}

pub fn iso(source: u8, gsi: u32, flags: u16) -> Vec<u8> {
    let mut e = std::vec![2, 10, 0, source];
    e.extend_from_slice(&gsi.to_le_bytes());
    e.extend_from_slice(&flags.to_le_bytes());
    e
}

pub fn nmi_source(flags: u16, gsi: u32) -> Vec<u8> {
    let mut e = std::vec![3, 8];
    e.extend_from_slice(&flags.to_le_bytes());
    e.extend_from_slice(&gsi.to_le_bytes());
    e
}

pub fn lapic_nmi(uid: u8, flags: u16, lint: u8) -> Vec<u8> {
    let mut e = std::vec![4, 6, uid];
    e.extend_from_slice(&flags.to_le_bytes());
    e.push(lint);
    e
}

pub fn lapic_override(address: u64) -> Vec<u8> {
    let mut e = std::vec![5, 12, 0, 0];
    e.extend_from_slice(&address.to_le_bytes());
    e
}

pub fn x2apic(id: u32, flags: u32, uid: u32) -> Vec<u8> {
    let mut e = std::vec![9, 16, 0, 0];
    e.extend_from_slice(&id.to_le_bytes());
    e.extend_from_slice(&flags.to_le_bytes());
    e.extend_from_slice(&uid.to_le_bytes());
    e
}

pub fn x2apic_nmi(flags: u16, uid: u32, lint: u8) -> Vec<u8> {
    let mut e = std::vec![0xa, 12];
    e.extend_from_slice(&flags.to_le_bytes());
    e.extend_from_slice(&uid.to_le_bytes());
    e.extend_from_slice(&[lint, 0, 0, 0]);
    e
}

pub fn madt_body(lapic_address: u32, flags: u32, entries: &[Vec<u8>]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&lapic_address.to_le_bytes());
    b.extend_from_slice(&flags.to_le_bytes());
    for e in entries {
        b.extend_from_slice(e);
    }
    b
}

/// Two CPUs, one I/O APIC, the usual ISA overrides: what a small PC has.
pub fn madt_qemu() -> Vec<u8> {
    madt_body(
        0xfee0_0000,
        1,
        &[
            lapic(0, 0, 1),
            lapic(1, 1, 1),
            ioapic(0, 0xfec0_0000, 0),
            iso(0, 2, 0),
            iso(9, 9, 0b1101),
            lapic_nmi(0xff, 0, 1),
        ],
    )
}

/// One of every entry type, plus an unknown one.
pub fn madt_full() -> Vec<u8> {
    madt_body(
        0xfee0_0000,
        0,
        &[
            lapic(0, 0, 1),
            lapic(1, 2, 0),
            lapic(2, 4, 2),
            x2apic(0x100, 1, 0x1000),
            ioapic(0, 0xfec0_0000, 0),
            ioapic(1, 0xfec0_1000, 24),
            iso(0, 2, 0),
            nmi_source(0b1111, 30),
            lapic_nmi(0xff, 0b0101, 1),
            x2apic_nmi(0, 0xffff_ffff, 1),
            std::vec![0x7f, 4, 0xaa, 0xbb],
            lapic_override(0x1_fee0_0000),
        ],
    )
}

/// Vendor 0x8086, legacy-replacement and 64-bit capable, 3 comparators,
/// revision 1; registers at 0xfed00000.
pub fn hpet_body() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&0x8086_a201u32.to_le_bytes());
    b.extend_from_slice(&[0, 64, 0, 0]);
    b.extend_from_slice(&0xfed0_0000u64.to_le_bytes());
    b.push(0);
    b.extend_from_slice(&0x80u16.to_le_bytes());
    b.push(0);
    b
}

pub fn mcfg_body(regions: &[(u64, u16, u8, u8)]) -> Vec<u8> {
    let mut b = std::vec![0u8; 8];
    for &(base, segment, start, end) in regions {
        b.extend_from_slice(&base.to_le_bytes());
        b.extend_from_slice(&segment.to_le_bytes());
        b.extend_from_slice(&[start, end, 0, 0, 0, 0]);
    }
    b
}

/// A FADT of `len` bytes (116 = ACPI 1.0, 276 = ACPI 6.x) describing a
/// PC-like machine. Fields that do not fit are left out.
pub fn fadt_table(dsdt: u64, len: usize) -> Vec<u8> {
    let revision = if len >= FADT_FULL_LEN { 6 } else { 1 };
    let mut t = table(b"FACP", revision, &std::vec![0u8; len.saturating_sub(SdtHeader::LEN)]);
    if let Ok(d) = u32::try_from(dsdt) {
        put(&mut t, DSDT, &d.to_le_bytes());
    }
    put(&mut t, SCI_INT, &9u16.to_le_bytes());
    put(&mut t, SMI_CMD, &0xb2u32.to_le_bytes());
    put(&mut t, PM_TMR_BLK, &0x608u32.to_le_bytes());
    put(&mut t, PM_TMR_LEN, &[4]);
    put(&mut t, CENTURY, &[0x32]);
    put(&mut t, IAPC_BOOT_ARCH, &0b11u16.to_le_bytes());
    put(&mut t, FLAGS, &FadtFlags::RESET_REG_SUP.to_le_bytes());
    put(&mut t, RESET_REG, &[1, 8, 0, 1]);
    put(&mut t, RESET_REG + 4, &0xcf9u64.to_le_bytes());
    put(&mut t, RESET_VALUE, &[6]);
    put(&mut t, FADT_MINOR_VERSION, &[5]);
    put(&mut t, X_DSDT, &dsdt.to_le_bytes());
    put(&mut t, X_PM_TMR_BLK, &[1, 32, 0, 3]);
    put(&mut t, X_PM_TMR_BLK + 4, &0x608u64.to_le_bytes());
    fix_checksum(&mut t);
    t
}

/// Physical memory made of byte buffers at fixed addresses.
#[derive(Debug, Default)]
pub struct Mem {
    regions: Vec<(u64, Vec<u8>)>,
}

impl Mem {
    pub fn add(&mut self, phys: u64, bytes: Vec<u8>) {
        self.regions.push((phys, bytes));
    }

    fn locate(&mut self, phys: u64) -> (&mut Vec<u8>, usize) {
        let (base, b) = self
            .regions
            .iter_mut()
            .find(|(base, b)| phys >= *base && phys - *base < b.len() as u64)
            .expect("address not in test memory");
        let off = (phys - *base) as usize;
        (b, off)
    }

    pub fn set(&mut self, phys: u64, v: u8) {
        let (b, off) = self.locate(phys);
        b[off] = v;
    }

    /// Flip bits in one byte, breaking the checksum around it.
    pub fn corrupt(&mut self, phys: u64) {
        let (b, off) = self.locate(phys);
        b[off] ^= 0x5a;
    }

    /// Size of the buffer that starts at `phys`.
    pub fn len_at(&self, phys: u64) -> usize {
        self.regions.iter().find(|(base, _)| *base == phys).map_or(0, |(_, b)| b.len())
    }

    /// Repair the SDT checksum of the buffer that starts at `phys`.
    pub fn fix_checksum(&mut self, phys: u64) {
        fix_checksum(self.locate(phys).0);
    }

    pub fn ptr(&self, phys: u64) -> *const u8 {
        self.bytes(phys, 1).expect("address not in test memory").as_ptr()
    }
}

impl PhysMemory for Mem {
    fn bytes(&self, phys: u64, len: usize) -> Option<&[u8]> {
        self.regions.iter().find_map(|(base, b)| {
            let off = usize::try_from(phys.checked_sub(*base)?).ok()?;
            b.get(off..off.checked_add(len)?)
        })
    }
}

/// Builds a whole firmware table set in a [`Mem`]: RSDP, RSDT and/or XSDT,
/// a FADT (always the first entry) pointing to a DSDT, then the extra tables.
#[derive(Debug)]
pub struct System {
    v1: bool,
    rsdt: bool,
    entries: Vec<Option<Vec<u8>>>,
    dangling: Vec<u64>,
}

/// A built [`System`] and where everything landed.
#[derive(Debug)]
pub struct Built {
    pub mem: Mem,
    pub rsdp: u64,
    pub rsdt: Option<u64>,
    pub xsdt: Option<u64>,
    pub dsdt: u64,
    /// Addresses listed by the root table(s), in order.
    pub tables: Vec<u64>,
}

impl Built {
    /// Every address holding an SDT, for corruption tests.
    pub fn all_sdts(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.tables.iter().copied().filter(|&a| a < 0x7000_0000).collect();
        v.push(self.dsdt);
        v.extend(self.rsdt);
        v.extend(self.xsdt);
        v
    }
}

const RSDP_AT: u64 = 0xe_0000;
const DSDT_AT: u64 = 0x10_0000;
const TABLES_AT: u64 = 0x20_0000;
const RSDT_AT: u64 = 0x30_0000;
const XSDT_AT: u64 = 0x1_0000_0000;

impl System {
    pub fn new() -> Self {
        System {
            v1: false,
            rsdt: true,
            entries: std::vec![Some(fadt_table(DSDT_AT, FADT_FULL_LEN))],
            dangling: Vec::new(),
        }
    }

    /// ACPI 1.0: revision 0 RSDP, RSDT only.
    pub fn v1(mut self) -> Self {
        self.v1 = true;
        self
    }

    /// ACPI 2.0+ with an XSDT and no RSDT.
    pub fn without_rsdt(mut self) -> Self {
        self.rsdt = false;
        self
    }

    pub fn with(mut self, signature: &[u8; 4], body: Vec<u8>) -> Self {
        self.entries.push(Some(table(signature, 1, &body)));
        self
    }

    /// List an address with nothing mapped behind it.
    pub fn dangling(mut self, phys: u64) -> Self {
        self.entries.push(None);
        self.dangling.push(phys);
        self
    }

    pub fn build(self) -> Built {
        let mut mem = Mem::default();
        mem.add(DSDT_AT, table(b"DSDT", 2, DSDT_AML));
        let mut tables = Vec::new();
        let mut dangling = self.dangling.into_iter();
        for (i, t) in self.entries.into_iter().enumerate() {
            match t {
                Some(t) => {
                    let phys = TABLES_AT + 0x1000 * i as u64;
                    mem.add(phys, t);
                    tables.push(phys);
                }
                None => tables.push(dangling.next().unwrap()),
            }
        }
        let rsdt_body: Vec<u8> = tables.iter().flat_map(|&a| (a as u32).to_le_bytes()).collect();
        let xsdt_body: Vec<u8> = tables.iter().flat_map(|&a| a.to_le_bytes()).collect();
        let rsdt = (self.rsdt || self.v1).then_some(RSDT_AT);
        let xsdt = (!self.v1).then_some(XSDT_AT);
        if let Some(a) = rsdt {
            mem.add(a, table(b"RSDT", 1, &rsdt_body));
        }
        if let Some(a) = xsdt {
            mem.add(a, table(b"XSDT", 1, &xsdt_body));
        }
        let rsdt_addr = rsdt.unwrap_or(0) as u32;
        mem.add(RSDP_AT, if self.v1 { rsdp_v1(rsdt_addr) } else { rsdp_v2(rsdt_addr, XSDT_AT) });
        Built { mem, rsdp: RSDP_AT, rsdt, xsdt, dsdt: DSDT_AT, tables }
    }
}

/// FADT, MADT with every entry type, HPET and two ECAM windows.
pub fn full_system() -> System {
    System::new()
        .with(b"APIC", madt_full())
        .with(b"HPET", hpet_body())
        .with(b"MCFG", mcfg_body(&[(0xb000_0000, 0, 0, 255), (0xc000_0000, 1, 0, 7)]))
}

/// Call `f` on every prefix of `t` that still validates: first as a plain
/// prefix (the length field then points past the end), then with the length
/// field and checksum rewritten so that the short table is "valid" and the
/// typed decoder has to cope with it.
pub fn for_each_prefix(t: &[u8], mut f: impl FnMut(Sdt<'_>)) {
    for n in 0..=t.len() {
        let plain = &t[..n];
        assert!(Sdt::parse(plain).is_err() || n == t.len());
        let mut fixed = plain.to_vec();
        set_length(&mut fixed, n as u32);
        if let Ok(sdt) = Sdt::parse(&fixed) {
            f(sdt);
        }
    }
}

pub fn walk_madt(m: &Madt) {
    black_box((m.local_apic_address(), m.local_apic_address_field(), m.flags(), m.has_8259()));
    for e in m.entries() {
        black_box(e);
    }
    for p in m.processors() {
        black_box((p, p.enabled(), p.online_capable()));
    }
    black_box(m.io_apics().count());
    for o in m.interrupt_overrides() {
        black_box((o.flags.polarity(), o.flags.trigger_mode()));
    }
}

pub fn walk_hpet(h: &Hpet) {
    black_box((h.comparator_count(), h.counter_is_64bit(), h.hardware_revision()));
    black_box((h.legacy_replacement_capable(), h.pci_vendor_id(), h.base, h.minimum_tick));
}

pub fn walk_fadt(f: &Fadt) {
    black_box((f.dsdt_address(), f.facs_address(), f.sci_interrupt(), f.smi_command_port()));
    black_box((f.pm_timer(), f.century(), f.boot_arch(), f.flags(), f.reset(), f.minor_version()));
    black_box((f.preferred_pm_profile(), f.hardware_reduced()));
}

pub fn walk_mcfg(m: &Mcfg) {
    for r in m.regions() {
        black_box(r);
    }
}

/// Exercise everything reachable from the RSDP; must never panic.
pub fn walk_everything(sys: &Built) {
    let Ok(acpi) = Acpi::new(&sys.mem, sys.rsdp) else { return };
    for t in acpi.tables() {
        let _ = black_box(t);
    }
    if let Ok(m) = acpi.table::<Madt>() {
        walk_madt(&m);
    }
    if let Ok(h) = acpi.table::<Hpet>() {
        walk_hpet(&h);
    }
    if let Ok(f) = acpi.table::<Fadt>() {
        walk_fadt(&f);
    }
    if let Ok(m) = acpi.table::<Mcfg>() {
        walk_mcfg(&m);
    }
    let _ = black_box(acpi.dsdt());
}
