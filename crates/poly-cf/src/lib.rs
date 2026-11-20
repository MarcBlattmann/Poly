//! The logic of Poly's own CoreFoundation (ADR 0260), kept here so it is
//! tested on the host: string encodings, string comparison, the format
//! engine of `CFStringCreateWithFormat`, number conversions, XML property
//! lists, URL text, bundle paths and versions, hashes and descriptions. The
//! framework itself (`personalities/xnu/corefoundation`, built for
//! `x86_64-apple-darwin` and linked by `rust-lld`) holds the objects and
//! calls into this crate.
//!
//! Inputs: Apple's public CoreFoundation documentation and SDK headers (for
//! constants and layouts), the CoreFoundation spec sheets and visible tests
//! of `specs/macos/CoreFoundation.framework`, the Unicode Consortium's
//! public mapping tables and character data, RFC 3986 and RFC 4648. No part
//! of Apple's CoreFoundation, CF-Lite or swift-corelibs-foundation was read.

#![no_std]

extern crate alloc;

pub mod bundle;
pub mod compare;
pub mod encoding;
pub mod format;
pub mod number;
pub mod plist;
pub mod text;
pub mod url;
