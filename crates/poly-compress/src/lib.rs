//! Decompressors for compressed blocks (squashfs, ADR 0168): zlib, LZ4
//! blocks, LZO1X, xz (LZMA2) and zstd. Each takes one compressed block and
//! the most it may expand to (`max_out`), and returns the decoded bytes;
//! a stream that would expand further, or is malformed, is an [`Error`].
//!
//! Written from the formats' public descriptions (RFC 1950/1951, the LZ4
//! block format description, the LZO1X stream layout, the xz file format
//! and LZMA's published algorithm, RFC 8878). Safe Rust, no allocation
//! beyond the output and the decoders' own tables.
#![no_std]
extern crate alloc;

pub mod lz4;
pub mod lzma;
pub mod lzo;
pub mod xz;
pub mod zstd;

use alloc::vec::Vec;

/// Why a block was not decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input ended early, a code is invalid or a match reaches before
    /// the start of the data.
    Corrupt,
    /// The output would exceed the limit.
    TooLarge,
    /// A valid stream using a feature this decoder does not have.
    Unsupported,
}

/// A zlib stream (RFC 1950).
pub fn zlib(src: &[u8], max_out: usize) -> Result<Vec<u8>, Error> {
    poly_png::inflate::zlib_decompress(src, max_out).map_err(|e| match e {
        poly_png::InflateError::TooLarge => Error::TooLarge,
        _ => Error::Corrupt,
    })
}

#[cfg(test)]
mod tests;
