//! Processes of the system (ADR 0235): programs that run with a service account's token, the
//! LocalSystem token in particular, whoever caused them to start.
//!
//! The token is [`Token::service`]'s (`specs/windows/advapi32.dll/SERVICES.md` section 4: user
//! `S-1-5-18`, session 0, System integrity, Everyone and Authenticated Users; Learn: "LocalSystem
//! Account" for the privileges). Two kinds of process get it:
//!
//! - the service control manager's services ([`crate::scm`]), started by `services.exe` as their
//!   configuration's account says;
//! - Poly's own programs that play a system service of Windows and that a program's DLL starts on
//!   demand: the COM activation process `comlaunch.exe` (Windows' DCOM launcher and RPCSS run as
//!   LocalSystem from boot). A DLL asks for it with `PolyStartSystemProcess(Program)`
//!   ([`start_system_process`]); the kernel, not the caller, picks the program and the token,
//!   so a caller gains nothing it could use. The same call starts the Task Scheduler (`Schedule`)
//!   through the service manager for a caller that may not start that service itself.

use crate::abi;
use crate::kernel::{NtKernel, ProcessCreate};
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::status::*;
use poly_ntobj::{NtStatus, ObjectType, ThreadRef};
use poly_security::service::Account;
use poly_security::sid::Sid;
use poly_security::token::Token;

type Result<T> = core::result::Result<T, NtStatus>;

/// `PolyStartSystemProcess`'s programs.
pub mod program {
    /// The COM activation process, `C:\Windows\System32\comlaunch.exe` (ADR 0220), as LocalSystem.
    pub const COM_ACTIVATION: u64 = 1;
    /// The Task Scheduler: the `Schedule` service (ADR 0226), through the service manager.
    pub const TASK_SCHEDULER: u64 = 2;
}

/// The window station and desktop a system process starts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The station of its account's services (`Service-0x0-3e7$`), not visible (SERVICES.md
    /// section 4): the services.
    ServiceStation,
    /// Its creator's: the COM activation process, whose servers start on the client's desktop.
    Creator,
}

/// The descriptor of the COM activation process: the defaults of its token (SYSTEM full,
/// Administrators read and execute) with a System label that only stops writing up. The sheet
/// `rpcrt4.dll/I_RpcOpenClientProcess` (`activation_calls_open_activator`,
/// `standard_server_cannot_open_activator`): an elevated server opens the activation process for
/// `PROCESS_QUERY_LIMITED_INFORMATION` and `PROCESS_VM_READ`, a standard one gets 5.
const ACTIVATION_PROCESS_SDDL: &str = "S:(ML;;NW;;;SI)";

/// Start `command` (its first word the image, quoted or not; `%Name%` already expanded) as
/// `account`, with the group of service SID `service` when given, on `place`; suspended when
/// asked. The new process's id and first thread. The token's defaults make the descriptors of the
/// process and its thread, completed by `process_security`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_as<K: NtKernel>(
    k: &mut K,
    command: Vec<u16>,
    env: Vec<u16>,
    account: Account,
    service: Option<Sid>,
    place: Place,
    suspended: bool,
    process_security: Option<Vec<u8>>,
) -> Result<(u32, ThreadRef)> {
    use crate::io::consts::*;
    let caller = k.caller();
    let image = poly_scm::config::image_of(&command);
    let name = crate::threads::image_nt_name(&image)?;
    let attrs =
        abi::Attributes { root: None, name: Some(name), attributes: poly_ntobj::OBJ_CASE_INSENSITIVE, security: None };
    let section = crate::io::services::with_named(
        k,
        &attrs,
        FILE_EXECUTE | FILE_READ_DATA | SYNCHRONIZE,
        FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
        |sys, cookie| crate::image::ImageSection::from_reader(sys.io.image_file(cookie)?),
    )?;
    let image_text = alloc::string::String::from_utf16_lossy(&image);
    let mut params = crate::params::for_program(&image_text, &[], env);
    params.command_line = command;
    params.current_dir = crate::layout::SYSTEM32.encode_utf16().chain("\\".encode_utf16()).collect();
    let station: Vec<u16> = account.station_name().encode_utf16().collect();
    let (token_id, token_obj, desktop) = k.with_state(|sys, _, _| -> Result<_> {
        let mut token = Token::service(account, service, 0);
        // The kernel chose this token, not the caller: it may start this one process with it
        // although it is of a higher integrity than the caller's (the one-time mark the consent
        // broker uses too, used up by the start; ADR 0222's rule for tokens a caller gives).
        token.consented = true;
        let (id, obj) = sys.objects.new_token(token, caller.thread)?;
        sys.objects.assign_security(obj, ObjectType::Token, Some(id), None);
        let desktop = match place {
            Place::ServiceStation => sys.winsta.service_start(&mut sys.objects, &station, caller.thread),
            Place::Creator => sys.winsta.child_start(&mut sys.objects, caller.process, caller.thread, None),
        };
        Ok((id, obj, desktop))
    })?;
    let req = ProcessCreate {
        parent: None,
        image: section,
        params,
        process_access: poly_ntobj::access::PROCESS_ALL_ACCESS,
        thread_access: poly_ntobj::access::THREAD_ALL_ACCESS,
        inherit_handles: false,
        suspended,
        token: Some(token_id),
        inherit_only: None,
        jobs: Vec::new(),
        join_creator_jobs: false,
        mitigation: poly_kernelbase::mitigation::State::new(),
        desktop,
        error_mode: 0,
        process_security,
        thread_security: None,
    };
    let created = k.create_process(req);
    k.with_state(|sys, _, _| {
        let _ = sys.objects.dereference(token_obj);
        if let Ok(c) = &created {
            let _ = sys.objects.close(&caller, c.process);
            let _ = sys.objects.close(&caller, c.thread);
        }
    });
    created.map(|c| (c.client.process.0, c.client.thread))
}

/// `PolyStartSystemProcess(Program)` (ADR 0235): start one of the system's programs
/// ([`program`]) the way the system runs it. `STATUS_SUCCESS` once it is started (or starting, for
/// a service); the error of the start otherwise; `STATUS_INVALID_PARAMETER` for another number.
/// The caller still waits for the program to be ready (its pipe), as it did before.
pub fn start_system_process<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [which] = abi::args::<K, 1>(k, regs)?;
    match which {
        program::COM_ACTIVATION => {
            let mut command: Vec<u16> = Vec::new();
            command.push(u16::from(b'"'));
            command.extend(crate::layout::SYSTEM32.encode_utf16());
            command.extend("\\comlaunch.exe\"".encode_utf16());
            let processors = k.processor_count();
            let env = k.with_state(|sys, _, _| crate::scm::system_environment(sys, processors).to_utf16());
            let sd = poly_security::sdstr::parse(&ACTIVATION_PROCESS_SDDL.encode_utf16().collect::<Vec<u16>>())
                .map_err(|_| STATUS_INVALID_PARAMETER)?;
            spawn_as(k, command, env, Account::LocalSystem, None, Place::Creator, false, Some(sd))?;
            Ok(STATUS_SUCCESS)
        }
        program::TASK_SCHEDULER => crate::scm::start_for_system(k, &"Schedule".encode_utf16().collect::<Vec<u16>>()),
        _ => Err(STATUS_INVALID_PARAMETER),
    }
}

#[cfg(test)]
mod tests {
    use super::program;
    use crate::services::Service;
    use crate::tests::Fake;
    use poly_ntobj::status::*;

    #[test]
    fn only_the_listed_programs_start() {
        let mut f = Fake::new();
        assert_eq!(f.call(Service::PolyStartSystemProcess, &[0]), STATUS_INVALID_PARAMETER);
        assert_eq!(f.call(Service::PolyStartSystemProcess, &[3]), STATUS_INVALID_PARAMETER);
    }

    #[test]
    fn the_task_scheduler_starts_through_the_service_manager() {
        let mut f = Fake::new();
        // No `Schedule` service in this registry: the manager's error, as a status.
        assert_eq!(f.call(Service::PolyStartSystemProcess, &[program::TASK_SCHEDULER]), NtStatus(0xC000_0034));
        assert!(f.sys.scm.scm.host_pid().is_none());
    }
}
