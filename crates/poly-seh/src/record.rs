//! `EXCEPTION_RECORD` and `DISPATCHER_CONTEXT` (x64, `winnt.h`; Learn:
//! "EXCEPTION_RECORD structure", "x64 exception handling").
//!
//! ```text
//! EXCEPTION_RECORD (0x98 bytes)          DISPATCHER_CONTEXT (0x50 bytes)
//!   0x00 DWORD ExceptionCode               0x00 ControlPc
//!   0x04 DWORD ExceptionFlags              0x08 ImageBase
//!   0x08 PEXCEPTION_RECORD ExceptionRecord 0x10 FunctionEntry (PRUNTIME_FUNCTION)
//!   0x10 PVOID ExceptionAddress            0x18 EstablisherFrame
//!   0x18 DWORD NumberParameters            0x20 TargetIp
//!   0x20 ULONG_PTR ExceptionInformation[15]0x28 ContextRecord (PCONTEXT)
//!                                          0x30 LanguageHandler
//!                                          0x38 HandlerData
//!                                          0x40 HistoryTable
//!                                          0x48 DWORD ScopeIndex, 0x4C DWORD Fill0
//! ```

use crate::consts::EXCEPTION_MAXIMUM_PARAMETERS;

/// `sizeof(EXCEPTION_RECORD)` on x64.
pub const EXCEPTION_RECORD_SIZE: usize = 0x98;
/// `sizeof(DISPATCHER_CONTEXT)` on x64.
pub const DISPATCHER_CONTEXT_SIZE: usize = 0x50;

/// An exception: what happened, where, and up to 15 parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExceptionRecord {
    pub code: u32,
    pub flags: u32,
    /// `ExceptionRecord`: an associated record (a nested exception), as a
    /// user address.
    pub record: u64,
    /// `ExceptionAddress`: where the exception occurred.
    pub address: u64,
    pub number_parameters: u32,
    pub information: [u64; EXCEPTION_MAXIMUM_PARAMETERS],
}

/// A record that cannot be used: `NumberParameters` is above
/// `EXCEPTION_MAXIMUM_PARAMETERS`, or the bytes are too few.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadRecord;

impl ExceptionRecord {
    /// A record with `code` at `address` and the given parameters (at most
    /// 15 are kept).
    pub fn new(code: u32, flags: u32, address: u64, params: &[u64]) -> ExceptionRecord {
        let n = params.len().min(EXCEPTION_MAXIMUM_PARAMETERS);
        let mut information = [0u64; EXCEPTION_MAXIMUM_PARAMETERS];
        information[..n].copy_from_slice(&params[..n]);
        ExceptionRecord { code, flags, record: 0, address, number_parameters: n as u32, information }
    }

    /// The parameters `NumberParameters` says are valid.
    pub fn parameters(&self) -> &[u64] {
        let n = (self.number_parameters as usize).min(EXCEPTION_MAXIMUM_PARAMETERS);
        &self.information[..n]
    }

    pub fn to_bytes(&self) -> [u8; EXCEPTION_RECORD_SIZE] {
        let mut b = [0u8; EXCEPTION_RECORD_SIZE];
        b[0..4].copy_from_slice(&self.code.to_le_bytes());
        b[4..8].copy_from_slice(&self.flags.to_le_bytes());
        b[8..16].copy_from_slice(&self.record.to_le_bytes());
        b[16..24].copy_from_slice(&self.address.to_le_bytes());
        b[24..28].copy_from_slice(&self.number_parameters.to_le_bytes());
        for (i, v) in self.information.iter().enumerate() {
            b[32 + 8 * i..40 + 8 * i].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// Read a record. Every parameter slot is kept (the caller's bytes), but
    /// a `NumberParameters` above 15 is refused.
    pub fn from_bytes(b: &[u8]) -> Result<ExceptionRecord, BadRecord> {
        let b = b.get(..EXCEPTION_RECORD_SIZE).ok_or(BadRecord)?;
        let u32_at = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        let u64_at = |at: usize| {
            let mut v = [0u8; 8];
            v.copy_from_slice(&b[at..at + 8]);
            u64::from_le_bytes(v)
        };
        let number_parameters = u32_at(24);
        if number_parameters as usize > EXCEPTION_MAXIMUM_PARAMETERS {
            return Err(BadRecord);
        }
        let mut information = [0u64; EXCEPTION_MAXIMUM_PARAMETERS];
        for (i, v) in information.iter_mut().enumerate() {
            *v = u64_at(32 + 8 * i);
        }
        Ok(ExceptionRecord {
            code: u32_at(0),
            flags: u32_at(4),
            record: u64_at(8),
            address: u64_at(16),
            number_parameters,
            information,
        })
    }
}

/// `DISPATCHER_CONTEXT` as the language-specific handler receives it:
/// addresses, as they are in the handler's address space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DispatcherContext {
    pub control_pc: u64,
    pub image_base: u64,
    /// Address of the `RUNTIME_FUNCTION`.
    pub function_entry: u64,
    pub establisher_frame: u64,
    pub target_ip: u64,
    /// Address of the `CONTEXT` of the frame being dispatched or unwound.
    pub context_record: u64,
    pub language_handler: u64,
    pub handler_data: u64,
    pub history_table: u64,
    pub scope_index: u32,
}

impl DispatcherContext {
    pub fn to_bytes(&self) -> [u8; DISPATCHER_CONTEXT_SIZE] {
        let mut b = [0u8; DISPATCHER_CONTEXT_SIZE];
        let words = [
            self.control_pc,
            self.image_base,
            self.function_entry,
            self.establisher_frame,
            self.target_ip,
            self.context_record,
            self.language_handler,
            self.handler_data,
            self.history_table,
        ];
        for (i, w) in words.iter().enumerate() {
            b[8 * i..8 * i + 8].copy_from_slice(&w.to_le_bytes());
        }
        b[0x48..0x4C].copy_from_slice(&self.scope_index.to_le_bytes());
        b
    }
}
