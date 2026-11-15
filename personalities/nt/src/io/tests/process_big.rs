//! Tests: process creation from an executable bigger than the kernel heap's
//! 4 MiB allocation limit (ADR 0155). `create_user_process` used to read the
//! whole file into one buffer, which panicked the kernel for Notepad++,
//! WinMerge, Node.js and six other programs.

use super::process::{create_process, parameters_in};
use super::*;
use alloc::vec;

#[test]
fn create_user_process_starts_an_executable_bigger_than_the_heap_can_allocate() {
    use poly_ldr::builder::{DATA, PeBuilder, Section, TEXT, append_section};
    let mut file = PeBuilder {
        image_base: 0x1_4000_0000,
        entry: 0x1000,
        sections: vec![Section::new(".text", vec![0xC3], TEXT)],
        stack_reserve: 0x1_0000,
        stack_commit: 0x1000,
        ..PeBuilder::default()
    }
    .write();
    // A data section padding it to 24 MiB, as a big resource section does.
    let at = append_section(&mut file, ".pad", b"resource data", 24 << 20, DATA).unwrap();
    file.resize(at.file_len as usize, 0);
    assert!(file.len() > 4 << 20);
    let mut t = setup();
    t.vfs_create(b"/nt/c/big.exe", &file);
    let (params, want) = parameters_in(&mut t, "C:\\big.exe", "big.exe", "C:\\");
    let (s, ph, th) = create_process(&mut t, params, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert!(ph != 0 && th != 0);
    let (child, mem) = t.f.children.last_mut().unwrap();
    let mut pp = [0u8; 8];
    mem.read(child.peb + crate::params::PEB_PROCESS_PARAMETERS, &mut pp).unwrap();
    let got =
        crate::params::read(u64::from_le_bytes(pp), &mut |a, b| mem.read(a, b).map_err(|_| STATUS_ACCESS_VIOLATION));
    assert_eq!(got, Ok(want));
    t.close(th);
    t.close(ph);
}

#[test]
fn create_user_process_refuses_a_big_file_that_is_not_an_image() {
    let mut t = setup();
    let mut junk = alloc::vec![0u8; 12 << 20];
    junk[..2].copy_from_slice(b"MZ");
    t.vfs_create(b"/nt/c/junk.exe", &junk);
    let (p, _) = parameters_in(&mut t, "C:\\junk.exe", "junk", "");
    assert_eq!(create_process(&mut t, p, 0).0, STATUS_INVALID_IMAGE_FORMAT);
    assert!(t.f.children.is_empty());
}
