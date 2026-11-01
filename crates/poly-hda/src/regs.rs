//! Controller registers (HDA 1.0a §3.3): byte offsets into the memory BAR
//! and the bits the driver uses. The widths noted are the register widths;
//! access them at that width (several are bytes or words next to other
//! registers).

/// Global Capabilities, 16 bits (§3.3, offset 00h). See [`crate::Gcap`].
pub const GCAP: usize = 0x00;
/// Minor and major version, 8 bits each (§3.3, offsets 02h / 03h); 1.0
/// controllers report 0 and 1.
pub const VMIN: usize = 0x02;
pub const VMAJ: usize = 0x03;
/// Global Control, 32 bits (§3.3, offset 08h).
pub const GCTL: usize = 0x08;
/// Wake Enable, 16 bits (§3.3, offset 0Ch).
pub const WAKEEN: usize = 0x0C;
/// State Change Status, 16 bits (§3.3, offset 0Eh): bit n set when the
/// codec on SDI line n requested a status change (after reset: is present).
/// Write 1 to clear.
pub const STATESTS: usize = 0x0E;
/// Interrupt Control, 32 bits (§3.3, offset 20h).
pub const INTCTL: usize = 0x20;
/// Interrupt Status, 32 bits (§3.3, offset 24h), read only.
pub const INTSTS: usize = 0x24;
/// CORB Lower / Upper Base Address, 32 bits each (§3.3, offsets 40h / 44h).
pub const CORBLBASE: usize = 0x40;
pub const CORBUBASE: usize = 0x44;
/// CORB Write Pointer, 16 bits (§3.3, offset 48h): bits 7:0, the last
/// command entry software wrote.
pub const CORBWP: usize = 0x48;
/// CORB Read Pointer, 16 bits (§3.3, offset 4Ah): bits 7:0, the last entry
/// the controller fetched; bit 15 resets it.
pub const CORBRP: usize = 0x4A;
/// CORB Control, 8 bits (§3.3, offset 4Ch).
pub const CORBCTL: usize = 0x4C;
/// CORB Status, 8 bits (§3.3, offset 4Dh).
pub const CORBSTS: usize = 0x4D;
/// CORB Size, 8 bits (§3.3, offset 4Eh).
pub const CORBSIZE: usize = 0x4E;
/// RIRB Lower / Upper Base Address, 32 bits each (§3.3, offsets 50h / 54h).
pub const RIRBLBASE: usize = 0x50;
pub const RIRBUBASE: usize = 0x54;
/// RIRB Write Pointer, 16 bits (§3.3, offset 58h): bits 7:0, the last
/// response entry the controller wrote; writing bit 15 resets it to 0.
pub const RIRBWP: usize = 0x58;
/// Response Interrupt Count, 16 bits (§3.3, offset 5Ah): bits 7:0, 0 = 256.
pub const RINTCNT: usize = 0x5A;
/// RIRB Control, 8 bits (§3.3, offset 5Ch).
pub const RIRBCTL: usize = 0x5C;
/// RIRB Status, 8 bits (§3.3, offset 5Dh).
pub const RIRBSTS: usize = 0x5D;
/// RIRB Size, 8 bits (§3.3, offset 5Eh).
pub const RIRBSIZE: usize = 0x5E;
/// DMA Position Lower / Upper Base Address, 32 bits each (§3.3, offsets
/// 70h / 74h). Not used: positions come from SDnLPIB.
pub const DPLBASE: usize = 0x70;
pub const DPUBASE: usize = 0x74;

/// Stream descriptor `n` starts here (§3.3, offset 80h + 20h·n). Input
/// streams come first, then output, then bidirectional ones (§3.3, GCAP).
pub const SD_BASE: usize = 0x80;
pub const SD_STRIDE: usize = 0x20;

/// Byte offset of stream descriptor `index`.
pub const fn sd(index: usize) -> usize {
    SD_BASE + SD_STRIDE * index
}

/// Offsets inside a stream descriptor (§3.3, SDnCTL .. SDnBDPU).
pub mod sd {
    /// Stream Descriptor Control, 24 bits: bits 7:0 at +0, bits 23:16 at +2
    /// (byte accesses keep the two halves apart).
    pub const CTL: usize = 0x00;
    /// The upper control byte (bits 23:16 of SDnCTL).
    pub const CTL_HI: usize = 0x02;
    /// Stream Descriptor Status, 8 bits; status bits are write 1 to clear.
    pub const STS: usize = 0x03;
    /// Link Position in Buffer, 32 bits, read only.
    pub const LPIB: usize = 0x04;
    /// Cyclic Buffer Length in bytes, 32 bits.
    pub const CBL: usize = 0x08;
    /// Last Valid Index, 16 bits: bits 7:0, the last BDL entry.
    pub const LVI: usize = 0x0C;
    /// FIFO Size, 16 bits, read only.
    pub const FIFOS: usize = 0x10;
    /// Stream format, 16 bits (§3.7.1).
    pub const FMT: usize = 0x12;
    /// Buffer Descriptor List Pointer, lower and upper 32 bits.
    pub const BDPL: usize = 0x18;
    pub const BDPU: usize = 0x1C;
}

/// GCTL bits (§3.3, offset 08h).
pub mod gctl {
    /// Controller Reset: 0 holds the controller and link in reset.
    pub const CRST: u32 = 1 << 0;
    /// Flush Control.
    pub const FCNTRL: u32 = 1 << 1;
    /// Accept Unsolicited Response Enable.
    pub const UNSOL: u32 = 1 << 8;
}

/// INTCTL / INTSTS bits (§3.3, offsets 20h / 24h). Bits 29:0 are one per
/// stream descriptor (SIE in INTCTL, SIS in INTSTS).
pub mod int {
    /// Global Interrupt Enable / Status.
    pub const GLOBAL: u32 = 1 << 31;
    /// Controller Interrupt Enable / Status (RIRB, CORB memory error, codec
    /// state change).
    pub const CONTROLLER: u32 = 1 << 30;
    /// The stream bits.
    pub const STREAMS: u32 = (1 << 30) - 1;
}

/// CORBRP bits.
pub mod corbrp {
    /// CORB Read Pointer Reset.
    pub const RST: u16 = 1 << 15;
}

/// CORBCTL bits.
pub mod corbctl {
    /// CORB Memory Error Interrupt Enable.
    pub const CMEIE: u8 = 1 << 0;
    /// Enable CORB DMA Engine.
    pub const RUN: u8 = 1 << 1;
}

/// CORBSTS bits.
pub mod corbsts {
    /// CORB Memory Error Indication (write 1 to clear).
    pub const CMEI: u8 = 1 << 0;
}

/// CORBSIZE / RIRBSIZE layout: bits 7:4 the size capability, bits 1:0 the
/// size in use.
pub mod ringsize {
    /// Size capability: 2 entries.
    pub const CAP_2: u8 = 1 << 4;
    /// Size capability: 16 entries.
    pub const CAP_16: u8 = 1 << 5;
    /// Size capability: 256 entries.
    pub const CAP_256: u8 = 1 << 6;
    /// Size field values.
    pub const SIZE_2: u8 = 0b00;
    pub const SIZE_16: u8 = 0b01;
    pub const SIZE_256: u8 = 0b10;
    pub const SIZE_MASK: u8 = 0b11;
}

/// RIRBWP bits.
pub mod rirbwp {
    /// RIRB Write Pointer Reset (write only, reads 0).
    pub const RST: u16 = 1 << 15;
}

/// RIRBCTL bits.
pub mod rirbctl {
    /// Response Interrupt Control.
    pub const RINTCTL: u8 = 1 << 0;
    /// RIRB DMA Enable.
    pub const DMAEN: u8 = 1 << 1;
    /// Response Overrun Interrupt Control.
    pub const OIC: u8 = 1 << 2;
}

/// RIRBSTS bits (write 1 to clear).
pub mod rirbsts {
    /// Response Interrupt.
    pub const RINTFL: u8 = 1 << 0;
    /// Response Overrun Interrupt Status.
    pub const OIS: u8 = 1 << 2;
}

/// SDnCTL bits, low byte (§3.3, SDnCTL).
pub mod sdctl {
    /// Stream Reset: 1 enters reset, reads back 1 once the stream is in it.
    pub const SRST: u8 = 1 << 0;
    /// Stream Run.
    pub const RUN: u8 = 1 << 1;
    /// Interrupt On Completion Enable.
    pub const IOCE: u8 = 1 << 2;
    /// FIFO Error Interrupt Enable.
    pub const FEIE: u8 = 1 << 3;
    /// Descriptor Error Interrupt Enable.
    pub const DEIE: u8 = 1 << 4;
    /// Upper byte (bits 23:16): stream number in bits 23:20, i.e. bits 7:4
    /// of the byte at [`super::sd::CTL_HI`].
    pub const STRM_SHIFT: u8 = 4;
}

/// SDnSTS bits (§3.3, SDnSTS).
pub mod sdsts {
    /// Buffer Completion Interrupt Status.
    pub const BCIS: u8 = 1 << 2;
    /// FIFO Error.
    pub const FIFOE: u8 = 1 << 3;
    /// Descriptor Error.
    pub const DESE: u8 = 1 << 4;
    /// FIFO Ready (read only).
    pub const FIFORDY: u8 = 1 << 5;
    /// The write 1 to clear bits.
    pub const CLEAR: u8 = BCIS | FIFOE | DESE;
}

/// Bytes per CORB entry (a 32-bit command, §4.4.1) and RIRB entry (a
/// 32-bit response and a 32-bit extended word, §4.4.2).
pub const CORB_ENTRY: usize = 4;
pub const RIRB_ENTRY: usize = 8;

/// RIRB entry extended word (§4.4.2, Figure "RIRB Entry Format"): bits 3:0
/// the codec address the response came from, bit 4 set for an unsolicited
/// response.
pub mod rirb_ex {
    pub const CODEC_MASK: u32 = 0xF;
    pub const UNSOL: u32 = 1 << 4;
}

/// Bytes per buffer descriptor list entry (§3.6.3): 64-bit address, 32-bit
/// length, 32-bit flags with IOC in bit 0.
pub const BDL_ENTRY: usize = 16;
/// BDL entry flag: Interrupt On Completion.
pub const BDL_IOC: u32 = 1 << 0;

/// The alignment of the CORB, RIRB, BDL and of each BDL buffer (§3.3,
/// §3.6.2): 128 bytes.
pub const DMA_ALIGN: u64 = 128;
