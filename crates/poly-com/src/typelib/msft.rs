//! The `MSFT` type-library format, the one MIDL writes (Google's updater carries six such
//! libraries as `TYPELIB` resources).
//!
//! No public document describes this format. Its reader comes from a format sheet derived by black-
//! box observation (batch B2b-2 of the COM plan, `docs/windows/com-typelib-plan.md` §4); until then
//! [`read`] reports [`ReadError::Unsupported`] and the loader answers `TYPE_E_CANTLOADLIBRARY`.
//! The reader's contract is [`super::file::read`]'s: the bytes of the file or resource in, a
//! [`super::Library`] out. Nothing else of the loader changes when it is filled in.

use super::Library;
use super::file::ReadError;

/// The signature an `MSFT` file starts with (`MSFT` in ASCII). Recognising it is enough to route
/// the bytes here; the layout behind it is the format sheet's.
pub const SIGNATURE: &[u8; 4] = b"MSFT";

/// Whether `bytes` look like an `MSFT` library.
pub fn recognises(bytes: &[u8]) -> bool {
    bytes.starts_with(SIGNATURE)
}

/// The library in an `MSFT` file: not readable yet (B2b-2).
pub fn read(_bytes: &[u8]) -> Result<Library, ReadError> {
    Err(ReadError::Unsupported)
}
