//! The Objective-C sections of an image mapped in memory, found from its
//! Mach-O header through the load commands (`<mach-o/loader.h>`:
//! `mach_header_64` is 32 bytes; `LC_SEGMENT_64` commands carry
//! `section_64` records of 80 bytes: names at 0 and 16, address at 32,
//! size at 40). Section addresses are link-time addresses: the slide is
//! the header's address minus the address of the segment that maps file
//! offset 0.

use crate::Mem;

/// `MH_MAGIC_64` (`<mach-o/loader.h>`).
const MH_MAGIC_64: u32 = 0xfeed_facf;
/// `LC_SEGMENT_64`.
const LC_SEGMENT_64: u32 = 0x19;

/// One section: its address in memory and size in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Section {
    pub addr: u64,
    pub size: u64,
}

impl Section {
    /// The pointers the section holds.
    pub fn pointers(&self) -> impl Iterator<Item = u64> + use<> {
        let (a, n) = (self.addr, self.size / 8);
        (0..n).map(move |i| a + 8 * i)
    }
}

/// The Objective-C sections of an image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjcSections {
    pub classlist: Section,
    /// Classes with `+load`.
    pub nlclslist: Section,
    pub catlist: Section,
    /// Categories with `+load`.
    pub nlcatlist: Section,
    pub selrefs: Section,
    pub protolist: Section,
    pub protorefs: Section,
    pub imageinfo: Section,
}

impl ObjcSections {
    /// Whether the image has any Objective-C metadata.
    pub fn is_empty(&self) -> bool {
        *self == ObjcSections::default()
    }
}

fn name16(m: &dyn Mem, at: u64) -> [u8; 16] {
    let mut b = [0u8; 16];
    for (i, x) in b.iter_mut().enumerate() {
        *x = m.u8(at + i as u64);
    }
    b
}

fn is(name: &[u8; 16], want: &[u8]) -> bool {
    name.starts_with(want) && name.get(want.len()).is_none_or(|&b| b == 0)
}

/// The Objective-C sections of the image whose Mach-O header is at
/// `header`; `None` if there is no 64-bit header there.
pub fn objc_sections(m: &dyn Mem, header: u64) -> Option<ObjcSections> {
    if m.u32(header) != MH_MAGIC_64 {
        return None;
    }
    let ncmds = m.u32(header + 16).min(4096);
    let mut at = header + 32;
    let mut slide = None;
    let mut out = ObjcSections::default();
    // First pass: the slide, from the segment that maps the header.
    let mut cmd = at;
    for _ in 0..ncmds {
        let (kind, size) = (m.u32(cmd), u64::from(m.u32(cmd + 4)));
        if kind == LC_SEGMENT_64 && m.u64(cmd + 40) == 0 && m.u64(cmd + 48) != 0 {
            slide = Some(header.wrapping_sub(m.u64(cmd + 24)));
        }
        if size < 8 {
            return None;
        }
        cmd += size;
    }
    let slide = slide.unwrap_or(0);
    for _ in 0..ncmds {
        let (kind, size) = (m.u32(at), u64::from(m.u32(at + 4)));
        if kind == LC_SEGMENT_64 {
            let nsects = u64::from(m.u32(at + 64)).min(256);
            for i in 0..nsects {
                let s = at + 72 + 80 * i;
                let name = name16(m, s);
                let sec = Section { addr: m.u64(s + 32).wrapping_add(slide), size: m.u64(s + 40) };
                let slot = match () {
                    () if is(&name, b"__objc_classlist") => &mut out.classlist,
                    () if is(&name, b"__objc_nlclslist") => &mut out.nlclslist,
                    () if is(&name, b"__objc_catlist") => &mut out.catlist,
                    () if is(&name, b"__objc_nlcatlist") => &mut out.nlcatlist,
                    () if is(&name, b"__objc_selrefs") => &mut out.selrefs,
                    () if is(&name, b"__objc_protolist") => &mut out.protolist,
                    () if is(&name, b"__objc_protorefs") => &mut out.protorefs,
                    () if is(&name, b"__objc_imageinfo") => &mut out.imageinfo,
                    () => continue,
                };
                *slot = sec;
            }
        }
        at += size;
    }
    Some(out)
}
