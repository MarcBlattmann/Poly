//! `NtDeviceIoControlFile` (Learn: "ZwDeviceIoControlFile"): Poly serves
//! it on console handles with the codes of [`poly_console::ioctl`] (ADR
//! 0080). Any other handle, or an unknown code, is
//! `STATUS_INVALID_DEVICE_REQUEST`; a code on the wrong kind of console
//! handle (a screen-buffer request on an input handle, say) is
//! `STATUS_INVALID_HANDLE`.

use super::{ConsoleKind, ConsoleTarget, Wait};
use crate::abi;
use crate::io::consts::STATUS_INVALID_DEVICE_REQUEST;
use crate::io::irp::IrpOp;
use crate::io::services::{MAX_CAPTURED_WRITE, Notify, file_handle, wait_sync, write_iosb};
use crate::kernel::NtKernel;
use crate::system::NtSystem;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_console::ioctl::*;
use poly_console::{InputRecord, consts::INPUT_RECORD_SIZE};
use poly_ntobj::status::*;
use poly_ntobj::{Caller, Handle, NtStatus, ObjectId};

type Result<T> = core::result::Result<T, NtStatus>;

/// What a request turned into.
enum Reply {
    /// Done in the call: the status and the output bytes.
    Data(NtStatus, Vec<u8>),
    /// A read completed at once (its completion delivers the data).
    Done,
    /// A read waiting for input.
    Pending { irp: u64, sync: bool },
}

fn u32_in(input: &[u8], at: usize) -> Result<u32> {
    input.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or(STATUS_INVALID_PARAMETER)
}

/// A Win32 error from the console's checks as a status.
fn invalid(_: u32) -> NtStatus {
    STATUS_INVALID_PARAMETER
}

fn need(t: ConsoleTarget, kind: ConsoleKind) -> Result<()> {
    if t.kind == kind { Ok(()) } else { Err(STATUS_INVALID_HANDLE) }
}

fn ok() -> Result<Reply> {
    Ok(Reply::Data(STATUS_SUCCESS, Vec::new()))
}

fn value(v: u32) -> Result<Reply> {
    Ok(Reply::Data(STATUS_SUCCESS, v.to_le_bytes().to_vec()))
}

/// `NtDeviceIoControlFile(FileHandle, Event, ApcRoutine, ApcContext,
/// IoStatusBlock*, IoControlCode, InputBuffer, InputBufferLength,
/// OutputBuffer, OutputBufferLength)`.
pub fn device_io_control_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, event, apc_routine, apc_context, iosb, code, inp, inlen, out, outlen] = abi::args::<K, 10>(k, regs)?;
    let (code, inlen, outlen) = (code as u32, inlen as u32 as usize, outlen as u32 as usize);
    if inlen > MAX_CAPTURED_WRITE {
        return Err(STATUS_INSUFFICIENT_RESOURCES);
    }
    let mut input = alloc::vec![0u8; inlen];
    if inlen > 0 {
        abi::read(k, inp, &mut input)?;
    }
    let n = Notify { event, apc_routine, apc_context, iosb };
    let caller = k.caller();
    // A socket handle: `\Device\Afd`'s requests (ADR 0095).
    let socket = k.with_state(|sys, _, _| -> Result<Option<(ObjectId, u64)>> {
        let file = file_handle(sys, &caller, Handle(h), 0)?;
        n.check_event(sys, &caller)?;
        Ok(crate::afd::services::is_endpoint(sys, file.1).then_some(file))
    })?;
    if let Some(file) = socket {
        return crate::afd::services::control(k, file, n, code, &input, out, outlen);
    }
    let (file, reply) = k.with_state(|sys, _, _| -> Result<((ObjectId, u64), Reply)> {
        let file = file_handle(sys, &caller, Handle(h), 0)?;
        n.check_event(sys, &caller)?;
        // A file on a volume: the file-system controls it answers (ADR 0091).
        if sys.io.is_disk(file.1) {
            let (status, data) = sys.io.fs_control(file.1, code, &input, outlen)?;
            return Ok((file, Reply::Data(status, data)));
        }
        let t = sys.io.console_target(file.1)?;
        let sync = sys.io.file(file.1).is_some_and(|f| f.is_sync());
        let r = control(sys, &caller, &n, file, t, sync, code, &input, out, outlen)?;
        Ok((file, r))
    })?;
    match reply {
        Reply::Data(status, data) => {
            let put = data.len().min(outlen);
            if put > 0 {
                abi::write(k, out, &data[..put])?;
            }
            write_iosb(k, iosb, status, put as u64)?;
            k.with_state(|sys, _, _| n.signal_done(sys, &caller, file, status, put as u64));
            Ok(status)
        }
        Reply::Done => Ok(STATUS_SUCCESS),
        Reply::Pending { irp, sync: true } => Ok(wait_sync(k, file.0, irp).0),
        Reply::Pending { .. } => Ok(STATUS_PENDING),
    }
}

#[allow(clippy::too_many_arguments)]
fn control(
    sys: &mut NtSystem,
    caller: &Caller,
    n: &Notify,
    file: (ObjectId, u64),
    t: ConsoleTarget,
    sync: bool,
    code: u32,
    input: &[u8],
    out: u64,
    outlen: usize,
) -> Result<Reply> {
    let io = &mut sys.io;
    let id = t.console;
    match code {
        GET_MODE => {
            let c = io.console(id).ok_or(STATUS_INVALID_HANDLE)?;
            match t.kind {
                ConsoleKind::Input => value(c.input_mode),
                ConsoleKind::Output => value(c.output_mode),
                ConsoleKind::Connection => Err(STATUS_INVALID_HANDLE),
            }
        }
        SET_MODE => {
            let m = u32_in(input, 0)?;
            let c = io.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?;
            match t.kind {
                ConsoleKind::Input => c.set_input_mode(m).map_err(invalid)?,
                ConsoleKind::Output => c.set_output_mode(m).map_err(invalid)?,
                ConsoleKind::Connection => return Err(STATUS_INVALID_HANDLE),
            }
            // A read waiting under the old mode may finish under the new one.
            io.retry_console(id);
            ok()
        }
        READ_W | READ_A => {
            need(t, ConsoleKind::Input)?;
            let unicode = code == READ_W;
            let max = if unicode { outlen / 2 } else { outlen };
            let irp = n.irp(sys, caller, file, sync, IrpOp::Read { offset: 0, len: outlen, buf: out, key: 0 })?;
            read_reply(sys.io.console_read(id, irp, Wait::Text { unicode, max, buf: out })?, sync)
        }
        READ_INPUT => {
            need(t, ConsoleKind::Input)?;
            io.want_input(id);
            let flags = u32_in(input, 0).unwrap_or(0);
            let max = outlen / INPUT_RECORD_SIZE;
            if flags & READ_INPUT_REMOVE != 0 && flags & READ_INPUT_WAIT != 0 && max > 0 {
                let irp = n.irp(sys, caller, file, sync, IrpOp::Read { offset: 0, len: outlen, buf: out, key: 0 })?;
                return read_reply(sys.io.console_read(id, irp, Wait::Input { max, buf: out })?, sync);
            }
            let c = io.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?;
            let records = c.read_input(max, flags & READ_INPUT_REMOVE != 0);
            Ok(Reply::Data(STATUS_SUCCESS, records.iter().flat_map(|r| r.to_bytes()).collect()))
        }
        INPUT_COUNT => {
            need(t, ConsoleKind::Input)?;
            io.want_input(id);
            value(io.console(id).ok_or(STATUS_INVALID_HANDLE)?.input_count() as u32)
        }
        FLUSH_INPUT => {
            need(t, ConsoleKind::Input)?;
            io.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?.flush_input();
            ok()
        }
        WRITE_W => {
            need(t, ConsoleKind::Output)?;
            if !input.len().is_multiple_of(2) {
                return Err(STATUS_INVALID_PARAMETER);
            }
            let units: Vec<u16> = input.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            value(io.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?.write_units(&units) as u32)
        }
        WRITE_A => {
            need(t, ConsoleKind::Output)?;
            value(io.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?.write_bytes(input) as u32)
        }
        GET_INFO => {
            need(t, ConsoleKind::Output)?;
            let info = io.console(id).ok_or(STATUS_INVALID_HANDLE)?.screen.info_ex();
            if outlen < info.len() {
                return Err(STATUS_BUFFER_TOO_SMALL);
            }
            Ok(Reply::Data(STATUS_SUCCESS, info.to_vec()))
        }
        SET_ATTRIBUTE => {
            need(t, ConsoleKind::Output)?;
            let a = u32_in(input, 0)?;
            io.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?.screen.set_attribute(a as u16);
            ok()
        }
        SET_CURSOR => {
            need(t, ConsoleKind::Output)?;
            let xy = u32_in(input, 0)?;
            let c = io.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?;
            c.screen.set_cursor(xy as u16 as i16, (xy >> 16) as u16 as i16).map_err(invalid)?;
            ok()
        }
        FILL => {
            need(t, ConsoleKind::Output)?;
            let f = Fill::from_bytes(input).ok_or(STATUS_INVALID_PARAMETER)?;
            let c = io.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?;
            let done = match f.kind {
                FILL_CHARACTER => c.screen.fill_char(f.value as u16, f.count, f.x, f.y),
                FILL_ATTRIBUTE => c.screen.fill_attr(f.value as u16, f.count, f.x, f.y),
                _ => return Err(STATUS_INVALID_PARAMETER),
            };
            value(done.map_err(invalid)?)
        }
        GET_CP => {
            let c = io.console(id).ok_or(STATUS_INVALID_HANDLE)?;
            value(if u32_in(input, 0)? == 0 { c.input_cp } else { c.output_cp })
        }
        SET_CP => {
            let (which, cp) = (u32_in(input, 0)?, u32_in(input, 4)?);
            io.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?.set_cp(cp, which != 0).map_err(invalid)?;
            ok()
        }
        GET_TITLE => {
            let c = io.console(id).ok_or(STATUS_INVALID_HANDLE)?;
            let bytes: Vec<u8> = c.title.iter().flat_map(|u| u.to_le_bytes()).collect();
            let status = if bytes.len() > outlen { STATUS_BUFFER_OVERFLOW } else { STATUS_SUCCESS };
            Ok(Reply::Data(status, bytes))
        }
        SET_TITLE => {
            if !input.len().is_multiple_of(2) {
                return Err(STATUS_INVALID_PARAMETER);
            }
            let units: Vec<u16> = input.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            io.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?.set_title(&units);
            ok()
        }
        SET_CTRL_ROUTINE => {
            need(t, ConsoleKind::Connection)?;
            let lo = u64::from(u32_in(input, 0)?);
            let hi = u64::from(u32_in(input, 4)?);
            io.set_ctrl_routine(file.1, lo | (hi << 32))?;
            ok()
        }
        GET_ID => value(id),
        GENERATE_CTRL => {
            io.console_generate_ctrl(id, u32_in(input, 0)?)?;
            ok()
        }
        WRITE_INPUT => {
            need(t, ConsoleKind::Input)?;
            let records: Vec<InputRecord> = input
                .chunks_exact(INPUT_RECORD_SIZE)
                .map(|c| InputRecord::from_bytes(c.try_into().expect("20 bytes")))
                .collect();
            io.console_records(id, &records);
            value(records.len() as u32)
        }
        _ => Err(STATUS_INVALID_DEVICE_REQUEST),
    }
}

fn read_reply(parked: Option<u64>, sync: bool) -> Result<Reply> {
    Ok(match parked {
        None => Reply::Done,
        Some(irp) => Reply::Pending { irp, sync },
    })
}
