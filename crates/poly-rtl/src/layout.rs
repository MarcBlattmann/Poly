//! Where the fields of the TEB, the PEB and `RTL_USER_PROCESS_PARAMETERS` are in a 64-bit and
//! in a 32-bit process (ADR 0186): one table per pointer width, so the kernel's process start,
//! `ntdll` and `kernelbase` (built for x64 and for i686 from the same sources) agree.
//!
//! Sources: `winternl.h` (`TEB.ProcessEnvironmentBlock`, `TEB.TlsSlots`, `PEB.BeingDebugged`,
//! `PEB.Ldr`, `PEB.ProcessParameters`, `RTL_USER_PROCESS_PARAMETERS.ImagePathName` and
//! `.CommandLine`), `winnt.h` (`NT_TIB`: `ExceptionList`, `StackBase`, `StackLimit`, `Self`; the
//! 32-bit `NtCurrentTeb` reads `fs:[0x18]`), the PE Format specification (the 32-bit
//! `ThreadLocalStoragePointer` at `FS:[0x2C]`), and the WDK's `ntpebteb.h` / `ntddk.h` for the
//! rest of the PEB and `RTL_USER_PROCESS_PARAMETERS` (each structure laid out with 4-byte or
//! 8-byte pointers). The x64 values are the ones `exec`, `params` and `peb` used before this
//! table existed. Fields marked *Poly's own* sit in space the headers call reserved (ADR 0063,
//! 0071, 0073, 0080, 0089); the 32-bit ones are placed the same way (the same `Reserved`
//! pointer slots, or after the documented fields), to be compared with Windows by the spec
//! agent.

/// The offsets for one pointer width. All offsets are in bytes from the structure's start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Bytes of a pointer (`PVOID`, `HANDLE`, `SIZE_T`): 4 or 8.
    pub ptr: u64,
    /// Bytes of a `UNICODE_STRING` (8 or 16); its `Buffer` is at [`Self::ptr`] (4 or 8).
    pub unicode_string: u64,

    // ----- TEB
    /// `NT_TIB.ExceptionList` (the SEH frame chain of a 32-bit thread).
    pub teb_exception_list: u64,
    pub teb_stack_base: u64,
    pub teb_stack_limit: u64,
    /// `NT_TIB.Self`.
    pub teb_self: u64,
    /// `TEB.ClientId`: the process id, then the thread id (two pointer-sized values).
    pub teb_client_id: u64,
    pub teb_tls_pointer: u64,
    pub teb_peb: u64,
    pub teb_deallocation_stack: u64,
    pub teb_tls_slots: u64,
    /// Bytes Poly allocates for a TEB (two pages; `ntdll` keeps its per-thread data at
    /// [`TEB_POLY`] in both).
    pub teb_size: u64,

    // ----- PEB
    pub peb_being_debugged: u64,
    pub peb_image_base: u64,
    pub peb_ldr: u64,
    pub peb_process_parameters: u64,
    pub peb_process_heap: u64,
    pub peb_number_of_processors: u64,
    pub peb_nt_global_flag: u64,
    pub peb_loader_lock: u64,
    pub peb_os_major_version: u64,
    pub peb_os_minor_version: u64,
    pub peb_os_build_number: u64,
    pub peb_os_csd_version: u64,
    pub peb_os_platform_id: u64,
    pub peb_image_subsystem: u64,
    pub peb_image_subsystem_major: u64,
    pub peb_image_subsystem_minor: u64,
    pub peb_session_id: u64,

    // ----- RTL_USER_PROCESS_PARAMETERS
    pub pp_maximum_length: u64,
    pub pp_length: u64,
    pub pp_flags: u64,
    /// *Poly's own*: the console connection (the first `Reserved2` pointer, `ConsoleHandle`).
    pub pp_console_handle: u64,
    /// *Poly's own*: the standard input, output and error handles (`Reserved2[2..5]`), one
    /// pointer apart.
    pub pp_standard_input: u64,
    /// `CurrentDirectory.DosPath`.
    pub pp_dos_current_directory: u64,
    pub pp_dll_path: u64,
    pub pp_image_path_name: u64,
    pub pp_command_line: u64,
    /// *Poly's own* use of `Environment`: the environment block.
    pub pp_environment: u64,
    /// *Poly's own*: the environment block's size in bytes, right after it.
    pub pp_environment_size: u64,
    /// *Poly's own*: the starting current directory, a `UNICODE_STRING`.
    pub pp_current_directory: u64,
    /// *Poly's own* (ADR 0089): the creator's `STARTUPINFO` (`cb` .. `cbReserved2`, 40 bytes),
    /// then `lpDesktop`, `lpTitle` and `lpReserved` as `UNICODE_STRING`s and the
    /// `lpReserved2` bytes' address.
    pub pp_startup: u64,
    pub pp_desktop: u64,
    pub pp_title: u64,
    pub pp_shell_info: u64,
    pub pp_reserved2: u64,
    /// `EnvironmentSize` (`SIZE_T`).
    pub pp_environment_size_field: u64,
}

/// Where `ntdll` keeps its own per-thread data in a TEB (ADR 0071), in both widths.
pub const TEB_POLY: u64 = 0x1840;
/// Where a 32-bit thread's "WOW64 file-system redirection is off" flag lives in its TEB (a `u32`,
/// zero in a new thread: redirection on): the last bytes of the two pages Poly allocates for a
/// TEB, far above [`TEB_POLY`]'s data. `kernelbase` writes it (`Wow64DisableWow64FsRedirection`),
/// the NT personality reads it before it redirects a file name (ADR 0186, stage 2).
pub const TEB_FS_REDIRECTION_OFF: u64 = 0x1FF0;
/// Bytes kept for `RTL_USER_PROCESS_PARAMETERS` before its strings.
pub const PARAMETERS_SIZE: u64 = 0x400;
/// Bytes of the `STARTUPINFO` fields copied into the parameters (`cb` .. `cbReserved2`).
pub const STARTUP_FIXED: u64 = 40;
/// `dwFlags` and `cbReserved2` within those bytes.
pub const STARTUP_FLAGS: u64 = 0x20;
pub const STARTUP_CB_RESERVED2: u64 = 0x26;

/// A 64-bit process.
pub const X64: Layout = Layout {
    ptr: 8,
    unicode_string: 16,
    teb_exception_list: 0x00,
    teb_stack_base: 0x08,
    teb_stack_limit: 0x10,
    teb_self: 0x30,
    teb_client_id: 0x40,
    teb_tls_pointer: 0x58,
    teb_peb: 0x60,
    teb_deallocation_stack: 0x1478,
    teb_tls_slots: 0x1480,
    teb_size: 0x2000,
    peb_being_debugged: 0x02,
    peb_image_base: 0x10,
    peb_ldr: 0x18,
    peb_process_parameters: 0x20,
    peb_process_heap: 0x30,
    peb_number_of_processors: 0xB8,
    peb_nt_global_flag: 0xBC,
    peb_loader_lock: 0x110,
    peb_os_major_version: 0x118,
    peb_os_minor_version: 0x11C,
    peb_os_build_number: 0x120,
    peb_os_csd_version: 0x122,
    peb_os_platform_id: 0x124,
    peb_image_subsystem: 0x128,
    peb_image_subsystem_major: 0x12C,
    peb_image_subsystem_minor: 0x130,
    peb_session_id: 0x2C0,
    pp_maximum_length: 0x00,
    pp_length: 0x04,
    pp_flags: 0x08,
    pp_console_handle: 0x10,
    pp_standard_input: 0x20,
    pp_dos_current_directory: 0x38,
    pp_dll_path: 0x50,
    pp_image_path_name: 0x60,
    pp_command_line: 0x70,
    pp_environment: 0x80,
    pp_environment_size: 0x88,
    pp_current_directory: 0x90,
    pp_startup: 0xA0,
    pp_desktop: 0xC8,
    pp_title: 0xD8,
    pp_shell_info: 0xE8,
    pp_reserved2: 0xF8,
    pp_environment_size_field: 0x3F0,
};

/// A 32-bit process (WOW64, ADR 0186).
pub const X86: Layout = Layout {
    ptr: 4,
    unicode_string: 8,
    teb_exception_list: 0x00,
    teb_stack_base: 0x04,
    teb_stack_limit: 0x08,
    teb_self: 0x18,
    teb_client_id: 0x20,
    teb_tls_pointer: 0x2C,
    teb_peb: 0x30,
    teb_deallocation_stack: 0xE0C,
    teb_tls_slots: 0xE10,
    teb_size: 0x2000,
    peb_being_debugged: 0x02,
    peb_image_base: 0x08,
    peb_ldr: 0x0C,
    peb_process_parameters: 0x10,
    peb_process_heap: 0x18,
    peb_number_of_processors: 0x64,
    peb_nt_global_flag: 0x68,
    peb_loader_lock: 0xA0,
    peb_os_major_version: 0xA4,
    peb_os_minor_version: 0xA8,
    peb_os_build_number: 0xAC,
    peb_os_csd_version: 0xAE,
    peb_os_platform_id: 0xB0,
    peb_image_subsystem: 0xB4,
    peb_image_subsystem_major: 0xB8,
    peb_image_subsystem_minor: 0xBC,
    peb_session_id: 0x1D4,
    pp_maximum_length: 0x00,
    pp_length: 0x04,
    pp_flags: 0x08,
    pp_console_handle: 0x10,
    pp_standard_input: 0x18,
    pp_dos_current_directory: 0x24,
    pp_dll_path: 0x30,
    pp_image_path_name: 0x38,
    pp_command_line: 0x40,
    pp_environment: 0x48,
    pp_environment_size: 0x4C,
    pp_current_directory: 0x50,
    pp_startup: 0x60,
    pp_desktop: 0x88,
    pp_title: 0x90,
    pp_shell_info: 0x98,
    pp_reserved2: 0xA0,
    pp_environment_size_field: 0x290,
};

/// The layout of the process this code is compiled for (`ntdll`, `kernelbase`).
#[cfg(target_pointer_width = "32")]
pub const NATIVE: Layout = X86;
/// The layout of the process this code is compiled for (`ntdll`, `kernelbase`).
#[cfg(not(target_pointer_width = "32"))]
pub const NATIVE: Layout = X64;

impl Layout {
    /// Little-endian bytes of a pointer-sized value.
    pub fn ptr_bytes(&self, v: u64) -> ([u8; 8], usize) {
        (v.to_le_bytes(), self.ptr as usize)
    }

    /// A `UNICODE_STRING` for `length` bytes (with room for a NUL in `MaximumLength`) at
    /// `buffer`: [`Self::unicode_string`] bytes.
    pub fn unicode(&self, length: u16, maximum: u16, buffer: u64) -> ([u8; 16], usize) {
        let mut b = [0u8; 16];
        b[0..2].copy_from_slice(&length.to_le_bytes());
        b[2..4].copy_from_slice(&maximum.to_le_bytes());
        let at = self.ptr as usize;
        b[at..at + self.ptr as usize].copy_from_slice(&buffer.to_le_bytes()[..self.ptr as usize]);
        (b, self.unicode_string as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_x86_offsets_follow_from_four_byte_pointers() {
        // winternl.h: TEB { PVOID Reserved1[12]; PPEB ProcessEnvironmentBlock; PVOID
        // Reserved2[399]; BYTE Reserved3[1952]; PVOID TlsSlots[64]; ... }.
        for l in [X86, X64] {
            assert_eq!(l.teb_peb, 12 * l.ptr);
            assert_eq!(l.teb_tls_slots, 13 * l.ptr + 399 * l.ptr + 1952);
            // PEB { BYTE Reserved1[2]; BYTE BeingDebugged; BYTE Reserved2[1]; PVOID
            // Reserved3[2]; PPEB_LDR_DATA Ldr; PRTL_USER_PROCESS_PARAMETERS ProcessParameters; }
            assert_eq!(l.peb_ldr, l.ptr + 2 * l.ptr);
            assert_eq!(l.peb_process_parameters, l.peb_ldr + l.ptr);
            // RTL_USER_PROCESS_PARAMETERS { BYTE Reserved1[16]; PVOID Reserved2[10];
            // UNICODE_STRING ImagePathName; UNICODE_STRING CommandLine; }
            assert_eq!(l.pp_image_path_name, 16 + 10 * l.ptr);
            assert_eq!(l.pp_command_line, l.pp_image_path_name + l.unicode_string);
            // NT_TIB: ExceptionList, StackBase, StackLimit, SubSystemTib, FiberData,
            // ArbitraryUserPointer, Self.
            assert_eq!(l.teb_self, 6 * l.ptr);
            assert_eq!(l.unicode_string, 2 * l.ptr);
        }
    }

    #[test]
    fn the_x86_parameters_fit_before_the_strings_and_do_not_overlap() {
        let l = X86;
        let mut fields = [
            (l.pp_console_handle, l.ptr),
            (l.pp_standard_input, 3 * l.ptr),
            (l.pp_dos_current_directory, l.unicode_string),
            (l.pp_dll_path, l.unicode_string),
            (l.pp_image_path_name, l.unicode_string),
            (l.pp_command_line, l.unicode_string),
            (l.pp_environment, l.ptr),
            (l.pp_environment_size, l.ptr),
            (l.pp_current_directory, l.unicode_string),
            (l.pp_startup, STARTUP_FIXED),
            (l.pp_desktop, l.unicode_string),
            (l.pp_title, l.unicode_string),
            (l.pp_shell_info, l.unicode_string),
            (l.pp_reserved2, l.ptr),
            (l.pp_environment_size_field, l.ptr),
        ];
        fields.sort();
        for w in fields.windows(2) {
            assert!(w[0].0 + w[0].1 <= w[1].0, "{w:x?}");
        }
        assert!(fields.last().map(|f| f.0 + f.1).unwrap() <= PARAMETERS_SIZE);
    }

    #[test]
    fn a_unicode_string_has_its_buffer_one_pointer_in() {
        let (b, n) = X86.unicode(4, 6, 0x1234_5678);
        assert_eq!(n, 8);
        assert_eq!(&b[..8], &[4, 0, 6, 0, 0x78, 0x56, 0x34, 0x12]);
        let (b, n) = X64.unicode(4, 6, 0x1234_5678);
        assert_eq!(n, 16);
        assert_eq!(&b[8..12], &[0x78, 0x56, 0x34, 0x12]);
    }
}
