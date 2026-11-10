use crate::{lz4, lzo};

#[test]
fn lz4_literals_and_overlapping_match() {
    // "ab" then a match of 6 at distance 2 ("ababab" + "ab"), then the
    // closing literals "!".
    let src = [0x22, b'a', b'b', 2, 0, 0x10, b'!'];
    assert_eq!(lz4::decompress(&src, 100).unwrap(), b"abababab!");
    assert_eq!(lz4::decompress(&src, 3), Err(crate::Error::TooLarge));
    assert_eq!(lz4::decompress(&[0x10, b'a', 5, 0, 0], 100), Err(crate::Error::Corrupt));
}

#[test]
fn lzo_literal_run_and_end_marker() {
    // First byte 17 + 3: three literals, then the end marker 0x11 0 0.
    let src = [20, b'x', b'y', b'z', 0x11, 0, 0];
    assert_eq!(lzo::decompress(&src, 10).unwrap(), b"xyz");
    assert_eq!(lzo::decompress(&[20, b'x'], 10), Err(crate::Error::Corrupt));
}
