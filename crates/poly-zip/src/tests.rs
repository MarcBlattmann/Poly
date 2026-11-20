use super::*;
extern crate std;
use alloc::vec;

/// A deflated file, a folder and a stored file, with an archive comment (`tests/fixtures/deflate.zip`,
/// written by Python's `zipfile`).
const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/deflate.zip");

#[test]
fn reads_a_deflated_and_a_stored_entry() {
    let a = Archive::parse(FIXTURE).unwrap();
    let names: Vec<&str> = a.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["lib/net45/a%20b.txt", "lib/net45/", "stored.bin"]);
    let text = a.find("LIB/NET45/A%20B.TXT").unwrap();
    assert_eq!(text.method, 8);
    let got = read(FIXTURE, text, 1 << 20).unwrap();
    assert_eq!(got, b"hello hello hello hello, deflated\n".repeat(20));
    assert!(a.entries[1].is_dir());
    let stored = read(FIXTURE, &a.entries[2], 1 << 20).unwrap();
    assert_eq!(stored, (0..=255u8).collect::<Vec<u8>>());
    assert_eq!(read(FIXTURE, text, 10), Err(Error::TooLarge));
    assert_eq!(part_name(&text.name), "lib/net45/a b.txt");
}

#[test]
fn a_damaged_entry_is_refused() {
    let mut bad = FIXTURE.to_vec();
    let a = Archive::parse(&bad).unwrap();
    let e = a.entries[2].clone();
    let at = e.local_offset as usize + data_start(&bad[e.local_offset as usize..]).unwrap() as usize;
    bad[at] ^= 0xFF;
    assert_eq!(read(&bad, &e, 1 << 20), Err(Error::BadChecksum));
    let mut wrong_method = e.clone();
    wrong_method.method = 12;
    assert_eq!(decode(&wrong_method, &[], 1 << 20), Err(Error::Unsupported));
    assert_eq!(Archive::parse(b"not a zip at all, longer than 22 bytes"), Err(Error::NotZip));
}

#[test]
fn the_directory_is_found_from_the_last_bytes_only() {
    let len = FIXTURE.len() as u64;
    let tail = &FIXTURE[FIXTURE.len() - 100..];
    let dir = central_directory(tail, len).unwrap();
    assert_eq!(dir.entries, 3);
    let cd = &FIXTURE[dir.offset as usize..(dir.offset + dir.size) as usize];
    assert_eq!(Archive::from_directory(cd, dir.entries).unwrap(), Archive::parse(FIXTURE).unwrap());
}

/// A stored archive with one entry written the ZIP64 way: 0xFFFFFFFF in the headers, the values in
/// the extra field, a ZIP64 end record and its locator (APPNOTE 4.3.14, 4.3.15, 4.5.3).
fn zip64(content: &[u8]) -> Vec<u8> {
    let crc = poly_png::crc32(&[content]);
    let name = b"big.bin";
    let mut z = Vec::new();
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    let le64 = |v: &mut Vec<u8>, x: u64| v.extend_from_slice(&x.to_le_bytes());
    // Local header with a ZIP64 extra field (sizes).
    le32(&mut z, LOCAL);
    le16(&mut z, 45);
    le16(&mut z, 0);
    le16(&mut z, 0);
    le32(&mut z, 0);
    le32(&mut z, crc);
    le32(&mut z, u32::MAX);
    le32(&mut z, u32::MAX);
    le16(&mut z, name.len() as u16);
    le16(&mut z, 20);
    z.extend_from_slice(name);
    le16(&mut z, 1);
    le16(&mut z, 16);
    le64(&mut z, content.len() as u64);
    le64(&mut z, content.len() as u64);
    z.extend_from_slice(content);
    let cd_start = z.len() as u64;
    le32(&mut z, CENTRAL);
    le16(&mut z, 45);
    le16(&mut z, 45);
    le16(&mut z, 0);
    le16(&mut z, 0);
    le32(&mut z, 0);
    le32(&mut z, crc);
    le32(&mut z, u32::MAX);
    le32(&mut z, u32::MAX);
    le16(&mut z, name.len() as u16);
    le16(&mut z, 28);
    le16(&mut z, 0);
    le16(&mut z, 0);
    le16(&mut z, 0);
    le32(&mut z, 0);
    le32(&mut z, u32::MAX);
    z.extend_from_slice(name);
    le16(&mut z, 1);
    le16(&mut z, 24);
    le64(&mut z, content.len() as u64);
    le64(&mut z, content.len() as u64);
    le64(&mut z, 0);
    let cd_size = z.len() as u64 - cd_start;
    let eocd64 = z.len() as u64;
    le32(&mut z, EOCD64);
    le64(&mut z, 44);
    le16(&mut z, 45);
    le16(&mut z, 45);
    le32(&mut z, 0);
    le32(&mut z, 0);
    le64(&mut z, 1);
    le64(&mut z, 1);
    le64(&mut z, cd_size);
    le64(&mut z, cd_start);
    le32(&mut z, EOCD64_LOCATOR);
    le32(&mut z, 0);
    le64(&mut z, eocd64);
    le32(&mut z, 1);
    le32(&mut z, EOCD);
    le16(&mut z, 0);
    le16(&mut z, 0);
    le16(&mut z, 0xFFFF);
    le16(&mut z, 0xFFFF);
    le32(&mut z, u32::MAX);
    le32(&mut z, u32::MAX);
    le16(&mut z, 0);
    z
}

#[test]
fn zip64_values_come_from_the_extra_field_and_the_zip64_end_record() {
    let content = vec![7u8; 1000];
    let z = zip64(&content);
    let a = Archive::parse(&z).unwrap();
    assert_eq!(a.entries.len(), 1);
    let e = &a.entries[0];
    assert_eq!((e.name.as_str(), e.size, e.compressed_size, e.local_offset), ("big.bin", 1000, 1000, 0));
    assert_eq!(read(&z, e, 1 << 20).unwrap(), content);
}

#[test]
fn part_names_are_percent_decoded() {
    assert_eq!(part_name("lib/net45/locales/en-US.pak"), "lib/net45/locales/en-US.pak");
    assert_eq!(part_name("a%20b%2Bc"), "a b+c");
    assert_eq!(part_name("100%"), "100%");
    assert_eq!(part_name("%zz"), "%zz");
    assert_eq!(part_name("caf%C3%A9"), "café");
}
