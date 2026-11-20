//! Tests: executables and DLLs bigger than the kernel heap can hold at once
//! (ADR 0155): only their headers are read when they are opened, and the
//! kernel says `STATUS_NO_MEMORY` when memory runs out instead of panicking.

use crate::image::{ImageFile, ImageSection, RelocTables, try_filled};
use crate::imageshare::FileIdentity;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use poly_ldr::builder::{DATA, PeBuilder, Section, TEXT, append_section, set_directory};
use poly_ntobj::status::{STATUS_INVALID_IMAGE_FORMAT, STATUS_NO_MEMORY};

/// The kernel heap's largest single allocation (`kernel/src/heap.rs`).
const HEAP_MAX_ALLOCATION: usize = 4 << 20;

/// A file `len` bytes long that holds `head` and zeros after it, and counts
/// what is read from it, without ever holding the zeros.
#[derive(Debug)]
struct Counting {
    head: Vec<u8>,
    len: u64,
    total: AtomicU64,
    largest: AtomicUsize,
}

impl Counting {
    fn new(head: Vec<u8>, len: u64) -> Arc<Counting> {
        Arc::new(Counting { head, len, total: AtomicU64::new(0), largest: AtomicUsize::new(0) })
    }

    fn read_total(&self) -> u64 {
        self.total.load(Ordering::SeqCst)
    }
}

impl ImageFile for Counting {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.len.saturating_sub(offset) as usize);
        buf[..n].fill(0);
        let have = (self.head.len() as u64).saturating_sub(offset) as usize;
        let k = n.min(have);
        if k > 0 {
            let from = offset as usize;
            buf[..k].copy_from_slice(&self.head[from..from + k]);
        }
        self.total.fetch_add(n as u64, Ordering::SeqCst);
        self.largest.fetch_max(n, Ordering::SeqCst);
        n
    }
}

/// A small x64 executable of `.text` (one `ret`) and a `.data` section.
fn small_exe() -> Vec<u8> {
    PeBuilder {
        image_base: 0x1_4000_0000,
        entry: 0x1000,
        sections: vec![Section::new(".text", vec![0xC3], TEXT), Section::new(".data", vec![1, 2, 3, 4], DATA)],
        stack_reserve: 0x1_0000,
        stack_commit: 0x1000,
        ..PeBuilder::default()
    }
    .write()
}

/// [`small_exe`] with a `.pad` data section of `pad` bytes, the way a big
/// resource or data section makes an executable big: the file's first bytes
/// (headers and the start of `.pad`), the length the file really has, and
/// where `.pad` starts.
fn padded_exe(pad: u32) -> (Vec<u8>, u64, u64) {
    let mut file = small_exe();
    let at = append_section(&mut file, ".pad", b"padding starts here", pad, DATA).expect("room in the headers");
    (file, at.file_len, u64::from(at.rva))
}

#[test]
fn a_big_executable_is_never_read_whole() {
    let (head, len, pad_rva) = padded_exe(64 << 20);
    assert!(len > 64 << 20);
    let file = Counting::new(head, len);
    let img = ImageSection::from_reader(file.clone()).expect("a 64 MiB executable opens");
    assert_eq!(img.size, len, "the image is as big as the file here");
    // Only the headers were read: one read of at most 64 KiB.
    assert!(file.read_total() <= 64 * 1024, "{} bytes read", file.read_total());
    assert!(file.largest.load(Ordering::SeqCst) <= 64 * 1024);
    assert!(file.largest.load(Ordering::SeqCst) < HEAP_MAX_ALLOCATION);
    // One page of `.pad` costs about one page of reads.
    let before = file.read_total();
    let mut page = [0xAAu8; 4096];
    assert_eq!(img.image.read_at(pad_rva, &mut page), 4096);
    assert_eq!(&page[..19], b"padding starts here");
    assert!(page[19..].iter().all(|&b| b == 0));
    assert!(file.read_total() - before <= 2 * 4096, "{} bytes read for one page", file.read_total() - before);
    // The last page of the image.
    let before = file.read_total();
    let mut last = [0xAAu8; 4096];
    assert_eq!(img.image.read_at(len - 4096, &mut last), 4096);
    assert!(last.iter().all(|&b| b == 0));
    assert!(file.read_total() - before <= 2 * 4096);
}

#[test]
fn an_image_of_almost_four_gib_opens_lazily() {
    // `SizeOfImage` near the 32-bit limit: the biggest thing made is the
    // image's page table (one entry per page), which fits the heap.
    let (head, len, _) = padded_exe(0xFFF0_0000);
    let file = Counting::new(head, len);
    let img = ImageSection::from_reader(file.clone()).expect("a 4 GiB image opens lazily");
    assert!(img.size > 0xFFF0_0000);
    assert!(file.read_total() <= 64 * 1024);
    assert_eq!(img.pages.len() as u64, img.size / 4096);
}

#[test]
fn a_big_file_that_is_not_a_pe_is_an_image_format_error() {
    let file = Counting::new(b"MZ but nothing else".to_vec(), 48 << 20);
    assert_eq!(ImageSection::from_reader(file.clone()).err(), Some(STATUS_INVALID_IMAGE_FORMAT));
    assert!(file.read_total() <= 64 * 1024);
}

#[test]
fn what_cannot_be_allocated_is_no_memory_not_a_panic() {
    // More than the address space can hold: `Vec::try_reserve_exact` says no.
    assert_eq!(try_filled(usize::MAX / 2, 0u8).err(), Some(STATUS_NO_MEMORY));
    assert_eq!(try_filled(isize::MAX as usize, 0u64).err(), Some(STATUS_NO_MEMORY));
    assert_eq!(try_filled(3, 7u8), Ok(vec![7, 7, 7]));
    assert_eq!(try_filled(0, 7u8), Ok(Vec::new()));
}

/// [`Counting`] that names its contents, as a file of a file system does.
#[derive(Debug)]
struct Named(Arc<Counting>);

impl ImageFile for Named {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> usize {
        self.0.read_at(offset, buf)
    }

    fn identity(&self) -> Option<FileIdentity> {
        Some(FileIdentity { dev: 1, ino: 42, size: self.0.len, mtime_sec: 7, mtime_nsec: 0, base: 0 })
    }
}

#[test]
fn views_of_a_file_relocated_to_the_same_base_share_their_pages() {
    // Two image sections of the same big file, as two processes make them.
    let (head, len, _) = padded_exe(64 << 20);
    let file = Counting::new(head, len);
    let one = ImageSection::from_reader(Arc::new(Named(file.clone()))).expect("opens");
    let two = ImageSection::from_reader(Arc::new(Named(file))).expect("opens");
    let at_base = one.image.share_key().expect("a file's image");
    assert_eq!(at_base.base, 0);
    let to = one.image_base() + 0x1000_0000;
    let a = one.relocated(to).expect("relocates").share_key().expect("a file's image");
    let b = two.relocated(to).expect("relocates").share_key().expect("a file's image");
    assert_eq!(a, b);
    assert_eq!(a, FileIdentity { base: to, ..at_base });
    let elsewhere = two.relocated(to + 0x10_0000).expect("relocates").share_key().expect("a file's image");
    assert_ne!(elsewhere, a);
    // A small image, held in memory: the same.
    let small = small_exe();
    let key = FileIdentity { dev: 1, ino: 43, size: small.len() as u64, mtime_sec: 7, mtime_nsec: 0, base: 0 };
    let held = ImageSection::laid_out(&small, Some(key)).expect("parses");
    assert_eq!(held.image.share_key(), Some(key));
    let moved = held.relocated(held.image_base() + 0x1000_0000).expect("relocates");
    assert_eq!(moved.share_key(), Some(FileIdentity { base: held.image_base() + 0x1000_0000, ..key }));
}

#[test]
fn relocating_a_big_image_reads_its_table_not_its_file() {
    // A table of 40,000 blocks of one entry each.
    let mut table = Vec::new();
    for i in 0..40_000u32 {
        table.extend_from_slice(&(0x1000 * (i % 100)).to_le_bytes());
        table.extend_from_slice(&12u32.to_le_bytes());
        table.extend_from_slice(&((10u16 << 12) | 8).to_le_bytes());
        table.extend_from_slice(&0u16.to_le_bytes());
    }
    let mut head = small_exe();
    let size = (table.len() as u32).next_multiple_of(4096);
    let rel = append_section(&mut head, ".rel2", &table, size, DATA).unwrap();
    // The table is not at the end of the data it was given: pad it to its size.
    head.resize(rel.file_len as usize, 0);
    let pad = append_section(&mut head, ".pad", &[], 160 << 20, DATA).unwrap();
    set_directory(&mut head, 5, rel.rva, table.len() as u32);
    let file = Counting::new(head, pad.file_len);
    let img = ImageSection::from_reader(file.clone()).expect("opens");
    let before = file.read_total();
    let moved = img.relocated(img.image_base() + 0x1000_0000).expect("the table is checked, not the file");
    assert_eq!(moved.len(), img.size);
    // About 8 bytes per block and its entry, twice: nothing like the 160 MB file.
    let read = file.read_total() - before;
    assert!(read < 4 << 20, "{read} bytes read to check the table");
}

/// ADR 0262: the second relocated view of a big image file, from a section of its own, takes the
/// table the first one checked instead of reading it again, and sees the same bytes.
#[test]
fn a_big_files_checked_relocation_table_is_kept_for_its_next_views() {
    let mut table = Vec::new();
    for i in 0..4_000u32 {
        table.extend_from_slice(&(0x1000 * (i % 100)).to_le_bytes());
        table.extend_from_slice(&12u32.to_le_bytes());
        table.extend_from_slice(&((10u16 << 12) | 8).to_le_bytes());
        table.extend_from_slice(&0u16.to_le_bytes());
    }
    let mut head = small_exe();
    let size = (table.len() as u32).next_multiple_of(4096);
    let rel = append_section(&mut head, ".rel2", &table, size, DATA).unwrap();
    head.resize(rel.file_len as usize, 0);
    let pad = append_section(&mut head, ".pad", &[], 160 << 20, DATA).unwrap();
    set_directory(&mut head, 5, rel.rva, table.len() as u32);
    let file = Counting::new(head, pad.file_len);
    let mut tables = RelocTables::default();
    let first = ImageSection::from_reader(Arc::new(Named(file.clone()))).expect("opens");
    let base = first.image_base() + 0x1000_0000;
    let a = first.relocated_from(base, &mut tables).expect("relocates");
    assert_eq!(tables.len(), 1);
    // Another section of the same file: the table is not read again.
    let second = ImageSection::from_reader(Arc::new(Named(file.clone()))).expect("opens");
    let before = file.read_total();
    let b = second.relocated_from(base + 0x10_0000, &mut tables).expect("relocates");
    assert_eq!(file.read_total() - before, 0, "the table was read again");
    assert_eq!(tables.len(), 1);
    // Both apply the fix-ups by their own delta: page 0 has 40 blocks of one entry, each fixing
    // the 8 bytes at RVA 8.
    let (mut x, mut y) = ([0u8; 8], [0u8; 8]);
    a.read_at(8, &mut x);
    b.read_at(8, &mut y);
    assert_eq!(u64::from_le_bytes(y).wrapping_sub(u64::from_le_bytes(x)), 40 * 0x10_0000);
}
