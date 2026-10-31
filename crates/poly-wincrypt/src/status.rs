//! The result codes our `bcrypt.dll` and `crypt32.dll` return: `NTSTATUS`
//! values (`ntstatus.h`; CNG returns them directly) and the Win32 errors /
//! `HRESULT`s CryptoAPI puts in the thread's last error (`winerror.h`).

/// An `NTSTATUS`.
pub type NtStatus = u32;

pub const STATUS_SUCCESS: NtStatus = 0;
pub const STATUS_INVALID_HANDLE: NtStatus = 0xC000_0008;
pub const STATUS_INVALID_PARAMETER: NtStatus = 0xC000_000D;
pub const STATUS_NO_MEMORY: NtStatus = 0xC000_0017;
pub const STATUS_BUFFER_TOO_SMALL: NtStatus = 0xC000_0023;
pub const STATUS_DATA_ERROR: NtStatus = 0xC000_003E;
pub const STATUS_NOT_SUPPORTED: NtStatus = 0xC000_00BB;
pub const STATUS_INTERNAL_ERROR: NtStatus = 0xC000_00E5;
pub const STATUS_INVALID_BUFFER_SIZE: NtStatus = 0xC000_0206;
pub const STATUS_NOT_FOUND: NtStatus = 0xC000_0225;
pub const STATUS_INVALID_SIGNATURE: NtStatus = 0xC000_A000;
pub const STATUS_AUTH_TAG_MISMATCH: NtStatus = 0xC000_A002;

// Win32 errors (`winerror.h`).
pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_INVALID_DATA: u32 = 13;
pub const ERROR_NOT_SUPPORTED: u32 = 50;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_MORE_DATA: u32 = 234;

// CryptoAPI HRESULTs (`winerror.h`).
pub const NTE_BAD_DATA: u32 = 0x8009_0005;
pub const NTE_BAD_SIGNATURE: u32 = 0x8009_0006;
pub const NTE_BAD_ALGID: u32 = 0x8009_0008;
pub const NTE_NOT_FOUND: u32 = 0x8009_0011;
pub const CRYPT_E_MSG_ERROR: u32 = 0x8009_1001;
pub const CRYPT_E_UNKNOWN_ALGO: u32 = 0x8009_1002;
pub const CRYPT_E_OID_FORMAT: u32 = 0x8009_1003;
pub const CRYPT_E_HASH_VALUE: u32 = 0x8009_1007;
pub const CRYPT_E_NOT_FOUND: u32 = 0x8009_2004;
pub const CRYPT_E_EXISTS: u32 = 0x8009_2005;
pub const CRYPT_E_SELF_SIGNED: u32 = 0x8009_2007;
pub const CRYPT_E_NO_MATCH: u32 = 0x8009_2009;
pub const CRYPT_E_UNEXPECTED_MSG_TYPE: u32 = 0x8009_200A;
pub const CRYPT_E_REVOKED: u32 = 0x8009_2010;
pub const CRYPT_E_NO_REVOCATION_CHECK: u32 = 0x8009_2012;
pub const CRYPT_E_REVOCATION_OFFLINE: u32 = 0x8009_2013;
pub const CRYPT_E_ASN1_ERROR: u32 = 0x8009_3100;
pub const CRYPT_E_ASN1_INTERNAL: u32 = 0x8009_3101;
pub const CRYPT_E_ASN1_EOD: u32 = 0x8009_3102;
pub const CRYPT_E_ASN1_CORRUPT: u32 = 0x8009_3103;
pub const CRYPT_E_ASN1_LARGE: u32 = 0x8009_3104;
pub const CRYPT_E_ASN1_BADTAG: u32 = 0x8009_310B;
pub const TRUST_E_CERT_SIGNATURE: u32 = 0x8009_6004;
pub const TRUST_E_BASIC_CONSTRAINTS: u32 = 0x8009_6019;
pub const CERT_E_EXPIRED: u32 = 0x800B_0101;
pub const CERT_E_VALIDITYPERIODNESTING: u32 = 0x800B_0102;
pub const CERT_E_ROLE: u32 = 0x800B_0103;
pub const CERT_E_PATHLENCONST: u32 = 0x800B_0104;
pub const CERT_E_CRITICAL: u32 = 0x800B_0105;
pub const CERT_E_PURPOSE: u32 = 0x800B_0106;
pub const CERT_E_UNTRUSTEDROOT: u32 = 0x800B_0109;
pub const CERT_E_CHAINING: u32 = 0x800B_010A;
pub const TRUST_E_FAIL: u32 = 0x800B_010B;
pub const CERT_E_REVOKED: u32 = 0x800B_010C;
pub const CERT_E_REVOCATION_FAILURE: u32 = 0x800B_010E;
pub const CERT_E_CN_NO_MATCH: u32 = 0x800B_010F;
pub const CERT_E_WRONG_USAGE: u32 = 0x800B_0110;
pub const CERT_E_INVALID_NAME: u32 = 0x800B_0114;
pub const E_INVALIDARG: u32 = 0x8007_0057;
