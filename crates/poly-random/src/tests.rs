//! Tests against the test vectors of RFC 8439, and of the generator's
//! construction.

extern crate std;

use super::*;
use std::vec::Vec;

/// Bytes from hex digits; whitespace and `:` are ignored.
fn hex(s: &str) -> Vec<u8> {
    let digits: Vec<u8> = s.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
    digits.chunks(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect()
}

fn key_0_to_31() -> [u8; 32] {
    core::array::from_fn(|i| i as u8)
}

#[test]
fn quarter_round_matches_rfc_8439_2_1_1() {
    let out = quarter_round(0x1111_1111, 0x0102_0304, 0x9b8d_6f43, 0x0123_4567);
    assert_eq!(out, (0xea2a_92f4, 0xcb1c_f8ce, 0x4581_472e, 0x5881_c4bb));
}

#[test]
fn state_setup_matches_rfc_8439_2_3_2() {
    let nonce: [u8; 12] = hex("000000090000004a00000000").try_into().unwrap();
    let s = initial_state(&key_0_to_31(), 1, &nonce);
    let expected = [
        0x6170_7865,
        0x3320_646e,
        0x7962_2d32,
        0x6b20_6574,
        0x0302_0100,
        0x0706_0504,
        0x0b0a_0908,
        0x0f0e_0d0c,
        0x1312_1110,
        0x1716_1514,
        0x1b1a_1918,
        0x1f1e_1d1c,
        0x0000_0001,
        0x0900_0000,
        0x4a00_0000,
        0x0000_0000,
    ];
    assert_eq!(s, expected);
}

#[test]
fn block_matches_rfc_8439_2_3_2() {
    let nonce: [u8; 12] = hex("000000090000004a00000000").try_into().unwrap();
    let expected = hex("10 f1 e7 e4 d1 3b 59 15 50 0f dd 1f a3 20 71 c4
                        c7 d1 f4 c7 33 c0 68 03 04 22 aa 9a c3 d4 6c 4e
                        d2 82 64 46 07 9f aa 09 14 c2 d7 05 d9 8b 02 a2
                        b5 12 9c d1 de 16 4e b9 cb d0 83 e8 a2 50 3c 4e");
    assert_eq!(block(&key_0_to_31(), 1, &nonce).as_slice(), expected.as_slice());
}

#[test]
fn block_matches_rfc_8439_appendix_a_1() {
    let zero = [0u8; 12];
    let v1 = hex("76 b8 e0 ad a0 f1 3d 90 40 5d 6a e5 53 86 bd 28
                  bd d2 19 b8 a0 8d ed 1a a8 36 ef cc 8b 77 0d c7
                  da 41 59 7c 51 57 48 8d 77 24 e0 3f b8 d8 4a 37
                  6a 43 b8 f4 15 18 a1 1c c3 87 b6 69 b2 ee 65 86");
    assert_eq!(block(&[0; 32], 0, &zero).as_slice(), v1.as_slice());
    let v2 = hex("9f 07 e7 be 55 51 38 7a 98 ba 97 7c 73 2d 08 0d
                  cb 0f 29 a0 48 e3 65 69 12 c6 53 3e 32 ee 7a ed
                  29 b7 21 76 9c e6 4e 43 d5 71 33 b0 74 d8 39 d5
                  31 ed 1f 28 51 0a fb 45 ac e1 0a 1f 4b 79 4d 6f");
    assert_eq!(block(&[0; 32], 1, &zero).as_slice(), v2.as_slice());
}

#[test]
fn cipher_matches_rfc_8439_2_4_2() {
    let nonce: [u8; 12] = hex("000000000000004a00000000").try_into().unwrap();
    let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let ciphertext = hex("6e 2e 35 9a 25 68 f9 80 41 ba 07 28 dd 0d 69 81
                          e9 7e 7a ec 1d 43 60 c2 0a 27 af cc fd 9f ae 0b
                          f9 1b 65 c5 52 47 33 ab 8f 59 3d ab cd 62 b3 57
                          16 39 d6 24 e6 51 52 ab 8f 53 0c 35 9f 08 61 d8
                          07 ca 0d bf 50 0d 6a 61 56 a3 8e 08 8a 22 b6 5e
                          52 bc 51 4d 16 cc f8 06 81 8c e9 1a b7 79 37 36
                          5a f9 0b bf 74 a3 5b e6 b4 0b 8e ed f2 78 5e 42
                          87 4d");
    let mut data = plaintext.to_vec();
    xor_keystream(&key_0_to_31(), 1, &nonce, &mut data);
    assert_eq!(data, ciphertext);
    // And back.
    xor_keystream(&key_0_to_31(), 1, &nonce, &mut data);
    assert_eq!(data.as_slice(), plaintext.as_slice());
}

/// What `fill` must produce from `key` for a piece of `len` bytes (at most
/// MAX_PER_KEY): the second half of block 0, then blocks 1.., and the next
/// key, the first half of block 0.
fn expected_piece(key: &[u8; 32], len: usize) -> (Vec<u8>, [u8; 32]) {
    let mut stream = Vec::new();
    for i in 0..64 {
        stream.extend_from_slice(&block(key, i, &[0; 12]));
    }
    (stream[32..32 + len].to_vec(), stream[..32].try_into().unwrap())
}

#[test]
fn fill_hands_out_the_keystream_after_the_next_key() {
    let mut g = Csprng::new();
    let (expected, _) = expected_piece(&[0; 32], 100);
    let mut out = [0u8; 100];
    g.fill(&mut out);
    assert_eq!(out.as_slice(), expected.as_slice());
}

#[test]
fn fill_rekeys_before_each_request_and_every_max_per_key_bytes() {
    let mut g = Csprng::new();
    let (piece1, key1) = expected_piece(&[0; 32], MAX_PER_KEY);
    let (piece2, key2) = expected_piece(&key1, 10);
    let mut out = std::vec![0u8; MAX_PER_KEY + 10];
    g.fill(&mut out);
    assert_eq!(&out[..MAX_PER_KEY], piece1.as_slice());
    assert_eq!(&out[MAX_PER_KEY..], piece2.as_slice());
    // The next request starts from the key after that.
    let (piece3, _) = expected_piece(&key2, 5);
    let mut more = [0u8; 5];
    g.fill(&mut more);
    assert_eq!(more.as_slice(), piece3.as_slice());
    assert_eq!(g.generation(), 3);
}

#[test]
fn consecutive_requests_differ() {
    let mut g = Csprng::new();
    g.add_entropy(b"some seed", 0);
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    g.fill(&mut a);
    g.fill(&mut b);
    assert_ne!(a, b);
    assert_ne!(a, [0; 32]);
}

#[test]
fn mixing_changes_the_output_and_is_deterministic() {
    let run = |inputs: &[&[u8]]| {
        let mut g = Csprng::new();
        for i in inputs {
            g.mix(i);
        }
        let mut out = [0u8; 48];
        g.fill(&mut out);
        out
    };
    assert_eq!(run(&[b"abc"]), run(&[b"abc"]));
    assert_ne!(run(&[b"abc"]), run(&[b"abd"]));
    assert_ne!(run(&[]), run(&[b""]), "even empty input changes the key");
    // Trailing zeros and chunk boundaries matter.
    assert_ne!(run(&[b"abc"]), run(&[b"abc\0"]));
    assert_ne!(run(&[&[1u8; 40]]), run(&[&[1u8; 32], &[1u8; 8]]));
    let long: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
    assert_ne!(run(&[&long]), run(&[&long[..999]]));
}

#[test]
fn readiness_counts_credited_bits_only() {
    let mut g = Csprng::new();
    assert!(!g.ready());
    g.mix(&[7; 64]);
    assert_eq!(g.credited_bits(), 0);
    g.add_entropy(&[1; 16], 128);
    assert!(!g.ready());
    g.add_entropy(&[2; 16], 200);
    assert!(g.ready());
    assert_eq!(g.credited_bits(), READY_BITS);
}

#[test]
fn output_bytes_look_uniform() {
    // A coarse sanity check of the generator as a whole: over 64 KiB every
    // byte value appears, and the bit balance is near one half.
    let mut g = Csprng::new();
    g.add_entropy(b"uniformity", 256);
    let mut out = std::vec![0u8; 65536];
    g.fill(&mut out);
    let mut counts = [0u32; 256];
    let mut ones = 0u64;
    for &b in &out {
        counts[b as usize] += 1;
        ones += u64::from(b.count_ones());
    }
    assert!(counts.iter().all(|&c| c > 150 && c < 370), "{counts:?}");
    let bits = out.len() as u64 * 8;
    assert!(ones.abs_diff(bits / 2) < bits / 100, "{ones} of {bits}");
}

#[test]
fn debug_never_shows_the_key() {
    let mut g = Csprng::new();
    g.add_entropy(&[0xab; 32], 256);
    let s = std::format!("{g:?}");
    assert!(!s.contains("key"), "{s}");
}
