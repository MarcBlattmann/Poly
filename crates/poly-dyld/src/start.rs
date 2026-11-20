//! The user-space half of Poly's dyld: the start code a dynamically linked
//! program's first thread runs before `main`, for x86-64 and for arm64 (run
//! by the translator, ADR 0257).
//!
//! The loader (in the kernel, using this crate) maps every image and applies
//! every fixup at exec time, so what is left for user space is small: call
//! the initializers, call `main(argc, argv, envp, apple)`, and end the
//! process with its result through libSystem's `exit` (the raw exit system
//! call if there is no libSystem). The start code is one page of x86-64
//! code; the page after it holds the [`StartData`] block it reads. Its
//! source is `stub/start_x86_64.s`; `stub/gen.sh` assembles it and prints
//! the bytes below.
//!
//! The code page also holds the thunk of thread-local variables (at
//! [`X86_64_TLV_GET`]): a descriptor's first word points to it, and code
//! calls it with the descriptor's address in `rdi` to get the variable's
//! address in `rax`. The key word of each descriptor is the address of the
//! image's block of thread-local storage, so the thunk returns key + offset
//! and changes no other register.
//!
//! The thread starts with the stack of [`crate::stack`] (the header address
//! below `argc`).
//!
//! The arm64 start code ([`ARM64_START`], `stub/start_arm64.s`) does the
//! same with the same data block at [`DATA_OFFSET`]; its thread-local thunk
//! (at [`ARM64_TLV_GET`]) takes the descriptor in `x0` and returns the
//! variable's address in `x0`, changing no other register.

use alloc::vec::Vec;

/// The x86-64 start code (`stub/start_x86_64.s`), then the thread-local
/// thunk at [`X86_64_TLV_GET`].
#[rustfmt::skip]
pub const X86_64_START: [u8; 169] = [
    0x48, 0x8d, 0x1d, 0xf9, 0xff, 0xff, 0xff, 0x48, 0x81, 0xc3, 0x00, 0x10,
    0x00, 0x00, 0x49, 0x89, 0xe4, 0x4d, 0x8b, 0x6c, 0x24, 0x08, 0x4d, 0x8d,
    0x74, 0x24, 0x10, 0x4f, 0x8d, 0x7c, 0xee, 0x08, 0x4c, 0x89, 0xfd, 0x48,
    0x83, 0x7d, 0x00, 0x00, 0x74, 0x06, 0x48, 0x83, 0xc5, 0x08, 0xeb, 0xf3,
    0x48, 0x83, 0xc5, 0x08, 0x48, 0x83, 0xe4, 0xf0, 0x6a, 0x00, 0x6a, 0x00,
    0x48, 0x8b, 0x04, 0x24, 0x48, 0x3b, 0x43, 0x18, 0x73, 0x1c, 0x48, 0x8b,
    0x44, 0xc3, 0x20, 0x48, 0xff, 0x04, 0x24, 0x4c, 0x89, 0xef, 0x4c, 0x89,
    0xf6, 0x4c, 0x89, 0xfa, 0x48, 0x89, 0xe9, 0x49, 0x89, 0xd8, 0xff, 0xd0,
    0xeb, 0xda, 0x48, 0x83, 0x7b, 0x08, 0x01, 0x75, 0x07, 0x49, 0x8d, 0x64,
    0x24, 0x08, 0xff, 0x23, 0x4c, 0x89, 0xef, 0x4c, 0x89, 0xf6, 0x4c, 0x89,
    0xfa, 0x48, 0x89, 0xe9, 0xff, 0x13, 0x89, 0xc7, 0x48, 0x8b, 0x43, 0x10,
    0x48, 0x85, 0xc0, 0x74, 0x02, 0xff, 0xd0, 0xb8, 0x01, 0x00, 0x00, 0x02,
    0x0f, 0x05, 0x0f, 0x0b, 0x66, 0x2e, 0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x66, 0x90, 0x48, 0x8b, 0x47, 0x08, 0x48, 0x03, 0x47, 0x10,
    0xc3,
];

/// The offset of the thread-local thunk in [`X86_64_START`].
pub const X86_64_TLV_GET: u64 = 0xa0;

/// The offset of the [`StartData`] block from the start code (the next
/// 4 KiB page).
pub const DATA_OFFSET: u64 = 0x1000;

/// The arm64 start code (`stub/start_arm64.s`) as instruction words, then
/// the thread-local thunk at [`ARM64_TLV_GET`].
#[rustfmt::skip]
pub const ARM64_START: [u32; 70] = [
    0x10000013, 0x91400673, 0x910003f4, 0xf9400695, 0x91004296, 0x8b150ed7,
    0x910022f7, 0xaa1703f8, 0xf8408709, 0xb5ffffe9, 0xd2800019, 0xf9400e69,
    0xeb09033f, 0x54000162, 0x9100826a, 0xf879794a, 0x91000739, 0xaa1503e0,
    0xaa1603e1, 0xaa1703e2, 0xaa1803e3, 0xd2800004, 0xd63f0140, 0x17fffff4,
    0xf9400669, 0xf100053f, 0x540000a1, 0x91002289, 0x9100013f, 0xf9400269,
    0xd61f0120, 0xaa1503e0, 0xaa1603e1, 0xaa1703e2, 0xaa1803e3, 0xf9400269,
    0xd63f0120, 0xf9400a69, 0xb4000049, 0xd63f0120, 0xd2800030, 0xd4001001,
    0xd4200020, 0xd503201f, 0xd503201f, 0xd503201f, 0xd503201f, 0xd503201f,
    0xd503201f, 0xd503201f, 0xd503201f, 0xd503201f, 0xd503201f, 0xd503201f,
    0xd503201f, 0xd503201f, 0xd503201f, 0xd503201f, 0xd503201f, 0xd503201f,
    0xd503201f, 0xd503201f, 0xd503201f, 0xd503201f, 0xf81f0fe1, 0xf9400801,
    0xf9400400, 0x8b010000, 0xf84107e1, 0xd65f03c0,
];

/// The offset of the thread-local thunk in [`ARM64_START`] (bytes).
pub const ARM64_TLV_GET: u64 = 0x100;

/// [`ARM64_START`] as little-endian bytes, to copy into memory.
pub fn arm64_start_bytes() -> Vec<u8> {
    ARM64_START.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// What the start code does after the initializers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Call `main(argc, argv, envp, apple)` (`LC_MAIN`), then exit with its
    /// result.
    CallMain = 0,
    /// Jump to an `LC_UNIXTHREAD` entry point with the stack pointer at
    /// `argc` (a program with its own start code that still uses dyld).
    Jump = 1,
}

/// The block the start code reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartData {
    /// `main`, or the entry point to jump to.
    pub entry: u64,
    pub mode: Mode,
    /// libSystem's `exit`, 0 for the raw exit system call.
    pub exit: u64,
    /// The initializers, in the order to call them, each with `(argc, argv,
    /// envp, apple, data)`: the fifth argument is the address of this block
    /// (ADR 0258), where libdyld finds [`StartData::images`].
    pub initializers: Vec<u64>,
    /// The loaded images in load order, the program first: what libdyld's
    /// `_dyld_image_count` and `_dyld_get_image_*` report (ADR 0258).
    pub images: Vec<ImageInfo>,
}

/// One loaded image, for libdyld.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    /// The address of its mapped Mach-O header.
    pub header: u64,
    /// How far it was moved from its preferred address.
    pub slide: u64,
    /// The path it was loaded from.
    pub path: alloc::string::String,
}

/// The word that starts the image table after the initializers: "PolyImgs".
pub const IMAGES_MAGIC: u64 = u64::from_le_bytes(*b"PolyImgs");

impl StartData {
    /// The block as the start code reads it: entry, mode, exit, count,
    /// then the initializers, 64-bit little-endian words. Then the image
    /// table: [`IMAGES_MAGIC`], the number of images, per image its header
    /// address, slide and the offset of its path from the start of the
    /// block; then the paths, each ending with a NUL.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(48 + 8 * self.initializers.len() + 24 * self.images.len());
        let words = [self.entry, self.mode as u64, self.exit, self.initializers.len() as u64];
        let table = [IMAGES_MAGIC, self.images.len() as u64];
        for w in words.into_iter().chain(self.initializers.iter().copied()).chain(table) {
            out.extend_from_slice(&w.to_le_bytes());
        }
        let mut path_at = (out.len() + 24 * self.images.len()) as u64;
        for im in &self.images {
            for w in [im.header, im.slide, path_at] {
                out.extend_from_slice(&w.to_le_bytes());
            }
            path_at += im.path.len() as u64 + 1;
        }
        for im in &self.images {
            out.extend_from_slice(im.path.as_bytes());
            out.push(0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_block_layout() {
        let images = alloc::vec![
            ImageInfo { header: 0x1_0000_0000, slide: 0, path: "/bin/a".into() },
            ImageInfo { header: 0x8_0000_0000, slide: 0x8_0000_0000, path: "/usr/lib/libSystem.B.dylib".into() },
        ];
        let d =
            StartData { entry: 0x1000, mode: Mode::CallMain, exit: 0x2000, initializers: alloc::vec![7, 8], images };
        let b = d.to_bytes();
        let w = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        assert_eq!((w(0), w(1), w(2), w(3), w(4), w(5)), (0x1000, 0, 0x2000, 2, 7, 8));
        assert_eq!((w(6), w(7)), (IMAGES_MAGIC, 2));
        assert_eq!((w(8), w(9), w(11), w(12)), (0x1_0000_0000, 0, 0x8_0000_0000, 0x8_0000_0000));
        let path = |at: u64| {
            let rest = &b[at as usize..];
            core::str::from_utf8(&rest[..rest.iter().position(|&c| c == 0).unwrap()]).unwrap()
        };
        assert_eq!(path(w(10)), "/bin/a");
        assert_eq!(path(w(13)), "/usr/lib/libSystem.B.dylib");
        assert_eq!(b.len() as u64, w(13) + 27);
    }

    #[test]
    fn code_fits_its_page_and_has_the_thunk() {
        assert!(X86_64_START.len() as u64 <= DATA_OFFSET);
        // mov 8(%rdi),%rax; add 16(%rdi),%rax; ret
        let t = &X86_64_START[X86_64_TLV_GET as usize..];
        assert_eq!(t, [0x48, 0x8b, 0x47, 0x08, 0x48, 0x03, 0x47, 0x10, 0xc3]);
        // The first instruction finds the code page: lea -7(%rip),%rbx.
        assert_eq!(&X86_64_START[..7], [0x48, 0x8d, 0x1d, 0xf9, 0xff, 0xff, 0xff]);
        // The raw exit call: mov $0x2000001,%eax; syscall.
        let exit = [0xb8, 0x01, 0x00, 0x00, 0x02, 0x0f, 0x05];
        assert!(X86_64_START.windows(7).any(|w| w == exit));
        // Initializers get the data block as their fifth argument: mov %rbx,%r8.
        assert!(X86_64_START.windows(3).any(|w| w == [0x49, 0x89, 0xd8]));
    }

    #[test]
    fn arm64_code_fits_and_has_the_thunk() {
        let bytes = arm64_start_bytes();
        assert!(bytes.len() as u64 <= DATA_OFFSET);
        // str x1, [sp, #-16]!; ldr x1, [x0, #16]; ldr x0, [x0, #8];
        // add x0, x0, x1; ldr x1, [sp], #16; ret
        let t = &ARM64_START[(ARM64_TLV_GET / 4) as usize..];
        assert_eq!(t, [0xf81f0fe1, 0xf9400801, 0xf9400400, 0x8b010000, 0xf84107e1, 0xd65f03c0]);
        // adr x19, .; add x19, x19, #0x1000: the data block.
        assert_eq!(ARM64_START[..2], [0x10000013, 0x91400673]);
        // The raw exit call: mov x16, #1; svc #0x80.
        assert!(ARM64_START.windows(2).any(|w| w == [0xd2800030, 0xd4001001]));
    }
}
