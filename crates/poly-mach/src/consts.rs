//! Constants of the Mach interface, from the public macOS SDK headers
//! (`MacOSX.sdk/usr/include/mach/*.h`). Only values and names are taken.

/// `kern_return_t` values (`mach/kern_return.h`).
pub mod kern {
    pub const KERN_SUCCESS: u32 = 0;
    pub const KERN_INVALID_ADDRESS: u32 = 1;
    pub const KERN_PROTECTION_FAILURE: u32 = 2;
    pub const KERN_NO_SPACE: u32 = 3;
    pub const KERN_INVALID_ARGUMENT: u32 = 4;
    pub const KERN_FAILURE: u32 = 5;
    pub const KERN_RESOURCE_SHORTAGE: u32 = 6;
    pub const KERN_MEMORY_ERROR: u32 = 10;
    pub const KERN_ALREADY_IN_SET: u32 = 11;
    pub const KERN_NOT_IN_SET: u32 = 12;
    pub const KERN_NAME_EXISTS: u32 = 13;
    pub const KERN_ABORTED: u32 = 14;
    pub const KERN_INVALID_NAME: u32 = 15;
    pub const KERN_INVALID_TASK: u32 = 16;
    pub const KERN_INVALID_RIGHT: u32 = 17;
    pub const KERN_INVALID_VALUE: u32 = 18;
    pub const KERN_UREFS_OVERFLOW: u32 = 19;
    pub const KERN_INVALID_CAPABILITY: u32 = 20;
    pub const KERN_RIGHT_EXISTS: u32 = 21;
    pub const KERN_TERMINATED: u32 = 37;
    pub const KERN_NOT_WAITING: u32 = 48;
    pub const KERN_OPERATION_TIMED_OUT: u32 = 49;
}

/// Port names, rights and types (`mach/port.h`).
pub mod port {
    pub const MACH_PORT_NULL: u32 = 0;
    pub const MACH_PORT_DEAD: u32 = 0xffff_ffff;

    pub const MACH_PORT_RIGHT_SEND: u32 = 0;
    pub const MACH_PORT_RIGHT_RECEIVE: u32 = 1;
    pub const MACH_PORT_RIGHT_SEND_ONCE: u32 = 2;
    pub const MACH_PORT_RIGHT_PORT_SET: u32 = 3;
    pub const MACH_PORT_RIGHT_DEAD_NAME: u32 = 4;

    /// `MACH_PORT_TYPE(right)`: `1 << (right + 16)`.
    pub const MACH_PORT_TYPE_SEND: u32 = 0x1_0000;
    pub const MACH_PORT_TYPE_RECEIVE: u32 = 0x2_0000;
    pub const MACH_PORT_TYPE_SEND_ONCE: u32 = 0x4_0000;
    pub const MACH_PORT_TYPE_PORT_SET: u32 = 0x8_0000;
    pub const MACH_PORT_TYPE_DEAD_NAME: u32 = 0x10_0000;

    /// `MACH_PORT_UREFS_MAX`.
    pub const MACH_PORT_UREFS_MAX: u32 = 0xffff;
    /// `MACH_PORT_QLIMIT_DEFAULT` (`MACH_PORT_QLIMIT_BASIC`).
    pub const MACH_PORT_QLIMIT_DEFAULT: usize = 5;
}

/// Messages (`mach/message.h`).
pub mod msg {
    pub const MACH_MSG_TYPE_MOVE_RECEIVE: u32 = 16;
    pub const MACH_MSG_TYPE_MOVE_SEND: u32 = 17;
    pub const MACH_MSG_TYPE_MOVE_SEND_ONCE: u32 = 18;
    pub const MACH_MSG_TYPE_COPY_SEND: u32 = 19;
    pub const MACH_MSG_TYPE_MAKE_SEND: u32 = 20;
    pub const MACH_MSG_TYPE_MAKE_SEND_ONCE: u32 = 21;
    /// What a receiver finds: `MACH_MSG_TYPE_PORT_RECEIVE` etc.
    pub const MACH_MSG_TYPE_PORT_RECEIVE: u32 = MACH_MSG_TYPE_MOVE_RECEIVE;
    pub const MACH_MSG_TYPE_PORT_SEND: u32 = MACH_MSG_TYPE_MOVE_SEND;
    pub const MACH_MSG_TYPE_PORT_SEND_ONCE: u32 = MACH_MSG_TYPE_MOVE_SEND_ONCE;

    pub const MACH_MSGH_BITS_REMOTE_MASK: u32 = 0x0000_001f;
    pub const MACH_MSGH_BITS_LOCAL_MASK: u32 = 0x0000_1f00;
    pub const MACH_MSGH_BITS_VOUCHER_MASK: u32 = 0x001f_0000;
    pub const MACH_MSGH_BITS_COMPLEX: u32 = 0x8000_0000;

    pub const MACH_MSG_PORT_DESCRIPTOR: u8 = 0;

    pub const MACH_SEND_MSG: u32 = 0x0000_0001;
    pub const MACH_RCV_MSG: u32 = 0x0000_0002;
    pub const MACH_RCV_LARGE: u32 = 0x0000_0004;
    pub const MACH_SEND_TIMEOUT: u32 = 0x0000_0010;
    pub const MACH_RCV_TIMEOUT: u32 = 0x0000_0100;
    /// `MACH_RCV_TRAILER_ELEMENTS(x)`: `(x & 0xf) << 24`.
    pub const MACH_RCV_TRAILER_ELEMENTS_SHIFT: u32 = 24;
    pub const MACH_MSG_TRAILER_FORMAT_0: u32 = 0;

    pub const MACH_MSG_SUCCESS: u32 = 0;
    pub const MACH_SEND_INVALID_DATA: u32 = 0x1000_0002;
    pub const MACH_SEND_INVALID_DEST: u32 = 0x1000_0003;
    pub const MACH_SEND_TIMED_OUT: u32 = 0x1000_0004;
    pub const MACH_SEND_INTERRUPTED: u32 = 0x1000_0007;
    pub const MACH_SEND_MSG_TOO_SMALL: u32 = 0x1000_0008;
    pub const MACH_SEND_INVALID_REPLY: u32 = 0x1000_0009;
    pub const MACH_SEND_INVALID_RIGHT: u32 = 0x1000_000a;
    pub const MACH_SEND_TOO_LARGE: u32 = 0x1000_000e;
    pub const MACH_SEND_INVALID_TYPE: u32 = 0x1000_000f;
    pub const MACH_SEND_INVALID_HEADER: u32 = 0x1000_0010;
    pub const MACH_RCV_INVALID_NAME: u32 = 0x1000_4002;
    pub const MACH_RCV_TIMED_OUT: u32 = 0x1000_4003;
    pub const MACH_RCV_TOO_LARGE: u32 = 0x1000_4004;
    pub const MACH_RCV_INTERRUPTED: u32 = 0x1000_4005;
    pub const MACH_RCV_INVALID_DATA: u32 = 0x1000_4008;
    pub const MACH_RCV_PORT_DIED: u32 = 0x1000_4009;

    /// The size of the trailer a receiver asked for with
    /// `MACH_RCV_TRAILER_ELEMENTS` (the trailer structs of `mach/message.h`:
    /// null 8, seqno 12, sender 20, audit 52, context 60, av and labels 68).
    pub const fn trailer_size(option: u32) -> u32 {
        match (option >> MACH_RCV_TRAILER_ELEMENTS_SHIFT) & 0xf {
            0 => 8,
            1 => 12,
            2 => 20,
            3 => 52,
            4 => 60,
            _ => 68,
        }
    }
}

/// `vm_allocate` flags (`mach/vm_statistics.h`).
pub mod vm {
    pub const VM_FLAGS_FIXED: u32 = 0x0000_0000;
    pub const VM_FLAGS_ANYWHERE: u32 = 0x0000_0001;
    pub const VM_FLAGS_OVERWRITE: u32 = 0x0000_4000;
    /// `VM_FLAGS_USER_ALLOCATE`: FIXED | ANYWHERE | PURGABLE | 4GB_CHUNK |
    /// RANDOM_ADDR | NO_CACHE | PERMANENT | OVERWRITE | GUARD_OBJECT_OPTOUT |
    /// TPRO | MTE | ALIAS_MASK.
    pub const VM_FLAGS_USER_ALLOCATE: u32 =
        0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x80 | 0x4000 | 0x40_0000 | 0x1000 | 0x2000 | 0xff00_0000;

    /// `vm_prot_t` bits (`mach/vm_prot.h`); the same values as `PROT_*`.
    pub const VM_PROT_READ: u32 = 1;
    pub const VM_PROT_WRITE: u32 = 2;
    pub const VM_PROT_EXECUTE: u32 = 4;
    pub const VM_PROT_ALL: u32 = 7;

    /// `vm_inherit_t` (`mach/vm_inherit.h`).
    pub const VM_INHERIT_SHARE: u32 = 0;
    pub const VM_INHERIT_COPY: u32 = 1;
    pub const VM_INHERIT_NONE: u32 = 2;
}

/// Mach trap numbers (`mach/mach_traps.h` names; numbers are ABI facts,
/// the `xnu-mach-traps` source of the coverage map).
pub mod trap {
    pub const MACH_VM_ALLOCATE: u64 = 10;
    pub const MACH_VM_DEALLOCATE: u64 = 12;
    pub const MACH_VM_PROTECT: u64 = 14;
    pub const MACH_PORT_ALLOCATE: u64 = 16;
    pub const MACH_PORT_DEALLOCATE: u64 = 18;
    pub const MACH_PORT_MOD_REFS: u64 = 19;
    pub const MACH_PORT_MOVE_MEMBER: u64 = 20;
    pub const MACH_PORT_INSERT_RIGHT: u64 = 21;
    pub const MACH_REPLY_PORT: u64 = 26;
    pub const THREAD_SELF: u64 = 27;
    pub const TASK_SELF: u64 = 28;
    pub const HOST_SELF: u64 = 29;
    pub const MACH_MSG: u64 = 31;
    pub const SEMAPHORE_SIGNAL: u64 = 33;
    pub const SEMAPHORE_SIGNAL_ALL: u64 = 34;
    pub const SEMAPHORE_SIGNAL_THREAD: u64 = 35;
    pub const SEMAPHORE_WAIT: u64 = 36;
    pub const SEMAPHORE_WAIT_SIGNAL: u64 = 37;
    pub const SEMAPHORE_TIMEDWAIT: u64 = 38;
    pub const SEMAPHORE_TIMEDWAIT_SIGNAL: u64 = 39;
    pub const MACH_PORT_TYPE: u64 = 76;
    pub const MACH_TIMEBASE_INFO: u64 = 89;
    pub const MACH_WAIT_UNTIL: u64 = 90;
}
