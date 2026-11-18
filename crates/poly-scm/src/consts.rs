//! Constants of the service functions (`winsvc.h`, `winnt.h`, `winerror.h`).

// ----- rights (winsvc.h; Learn: "Service Security and Access Rights")
pub const SC_MANAGER_CONNECT: u32 = 0x1;
pub const SC_MANAGER_CREATE_SERVICE: u32 = 0x2;
pub const SC_MANAGER_ENUMERATE_SERVICE: u32 = 0x4;
pub const SC_MANAGER_LOCK: u32 = 0x8;
pub const SC_MANAGER_QUERY_LOCK_STATUS: u32 = 0x10;
pub const SC_MANAGER_MODIFY_BOOT_CONFIG: u32 = 0x20;
pub const SC_MANAGER_ALL_ACCESS: u32 = 0xF003F;

pub const SERVICE_QUERY_CONFIG: u32 = 0x1;
pub const SERVICE_CHANGE_CONFIG: u32 = 0x2;
pub const SERVICE_QUERY_STATUS: u32 = 0x4;
pub const SERVICE_ENUMERATE_DEPENDENTS: u32 = 0x8;
pub const SERVICE_START: u32 = 0x10;
pub const SERVICE_STOP: u32 = 0x20;
pub const SERVICE_PAUSE_CONTINUE: u32 = 0x40;
pub const SERVICE_INTERROGATE: u32 = 0x80;
pub const SERVICE_USER_DEFINED_CONTROL: u32 = 0x100;
pub const SERVICE_ALL_ACCESS: u32 = 0xF01FF;

// winnt.h
pub const DELETE: u32 = 0x1_0000;
pub const READ_CONTROL: u32 = 0x2_0000;
pub const WRITE_DAC: u32 = 0x4_0000;
pub const WRITE_OWNER: u32 = 0x8_0000;
pub const ACCESS_SYSTEM_SECURITY: u32 = 0x100_0000;
pub const MAXIMUM_ALLOWED: u32 = 0x200_0000;
pub const STANDARD_RIGHTS_READ: u32 = READ_CONTROL;
pub const STANDARD_RIGHTS_WRITE: u32 = READ_CONTROL;
pub const STANDARD_RIGHTS_EXECUTE: u32 = READ_CONTROL;

// ----- service types, start types, error control
pub const SERVICE_KERNEL_DRIVER: u32 = 0x1;
pub const SERVICE_FILE_SYSTEM_DRIVER: u32 = 0x2;
pub const SERVICE_DRIVER: u32 = 0xB;
pub const SERVICE_WIN32_OWN_PROCESS: u32 = 0x10;
pub const SERVICE_WIN32_SHARE_PROCESS: u32 = 0x20;
pub const SERVICE_WIN32: u32 = 0x30;
pub const SERVICE_INTERACTIVE_PROCESS: u32 = 0x100;
pub const SERVICE_TYPE_ALL: u32 = 0x3FF;

pub const SERVICE_BOOT_START: u32 = 0;
pub const SERVICE_SYSTEM_START: u32 = 1;
pub const SERVICE_AUTO_START: u32 = 2;
pub const SERVICE_DEMAND_START: u32 = 3;
pub const SERVICE_DISABLED: u32 = 4;

pub const SERVICE_NO_CHANGE: u32 = 0xFFFF_FFFF;

// ----- states, controls, accepted controls
pub const SERVICE_STOPPED: u32 = 1;
pub const SERVICE_START_PENDING: u32 = 2;
pub const SERVICE_STOP_PENDING: u32 = 3;
pub const SERVICE_RUNNING: u32 = 4;
pub const SERVICE_CONTINUE_PENDING: u32 = 5;
pub const SERVICE_PAUSE_PENDING: u32 = 6;
pub const SERVICE_PAUSED: u32 = 7;

pub const SERVICE_CONTROL_STOP: u32 = 1;
pub const SERVICE_CONTROL_PAUSE: u32 = 2;
pub const SERVICE_CONTROL_CONTINUE: u32 = 3;
pub const SERVICE_CONTROL_INTERROGATE: u32 = 4;
pub const SERVICE_CONTROL_SHUTDOWN: u32 = 5;

pub const SERVICE_ACCEPT_STOP: u32 = 0x1;
pub const SERVICE_ACCEPT_PAUSE_CONTINUE: u32 = 0x2;
pub const SERVICE_ACCEPT_SHUTDOWN: u32 = 0x4;
/// Every `SERVICE_ACCEPT_*` bit `winsvc.h` defines (STOP to USERMODEREBOOT, `0x800`).
pub const SERVICE_ACCEPT_ALL: u32 = 0xFFF;

pub const SERVICE_ACTIVE: u32 = 1;
pub const SERVICE_INACTIVE: u32 = 2;
pub const SERVICE_STATE_ALL: u32 = 3;

// ----- ChangeServiceConfig2W levels
pub const SERVICE_CONFIG_DESCRIPTION: u32 = 1;
pub const SERVICE_CONFIG_FAILURE_ACTIONS: u32 = 2;
pub const SERVICE_CONFIG_DELAYED_AUTO_START_INFO: u32 = 3;
pub const SERVICE_CONFIG_FAILURE_ACTIONS_FLAG: u32 = 4;
pub const SERVICE_CONFIG_SERVICE_SID_INFO: u32 = 5;
pub const SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO: u32 = 6;
pub const SERVICE_CONFIG_PRESHUTDOWN_INFO: u32 = 7;
pub const SERVICE_CONFIG_TRIGGER_INFO: u32 = 8;

pub const SC_ACTION_NONE: u32 = 0;
pub const SC_ACTION_RESTART: u32 = 1;
pub const SC_ACTION_REBOOT: u32 = 2;
pub const SC_ACTION_RUN_COMMAND: u32 = 3;

/// The default preshutdown timeout (the sheet of `QueryServiceConfig2W`).
pub const DEFAULT_PRESHUTDOWN_MS: u32 = 10_000;

// ----- winerror.h
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_INVALID_DATA: u32 = 13;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_INVALID_NAME: u32 = 123;
pub const ERROR_INVALID_LEVEL: u32 = 124;
pub const ERROR_BAD_EXE_FORMAT: u32 = 193;
pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_INVALID_ADDRESS: u32 = 487;
pub const ERROR_DEPENDENT_SERVICES_RUNNING: u32 = 1051;
pub const ERROR_INVALID_SERVICE_CONTROL: u32 = 1052;
pub const ERROR_SERVICE_REQUEST_TIMEOUT: u32 = 1053;
pub const ERROR_SERVICE_ALREADY_RUNNING: u32 = 1056;
pub const ERROR_INVALID_SERVICE_ACCOUNT: u32 = 1057;
pub const ERROR_SERVICE_DISABLED: u32 = 1058;
pub const ERROR_CIRCULAR_DEPENDENCY: u32 = 1059;
pub const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
pub const ERROR_SERVICE_CANNOT_ACCEPT_CTRL: u32 = 1061;
pub const ERROR_SERVICE_NOT_ACTIVE: u32 = 1062;
pub const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: u32 = 1063;
pub const ERROR_DATABASE_DOES_NOT_EXIST: u32 = 1065;
pub const ERROR_SERVICE_SPECIFIC_ERROR: u32 = 1066;
pub const ERROR_PROCESS_ABORTED: u32 = 1067;
pub const ERROR_SERVICE_DEPENDENCY_FAIL: u32 = 1068;
pub const ERROR_SERVICE_MARKED_FOR_DELETE: u32 = 1072;
pub const ERROR_SERVICE_EXISTS: u32 = 1073;
pub const ERROR_SERVICE_DEPENDENCY_DELETED: u32 = 1075;
pub const ERROR_SERVICE_NEVER_STARTED: u32 = 1077;
pub const ERROR_DUPLICATE_SERVICE_NAME: u32 = 1078;
pub const ERROR_INVALID_SERVICE_LOCK: u32 = 1071;
pub const RPC_X_BAD_STUB_DATA: u32 = 1783;

/// The longest service and display name (units).
pub const MAX_NAME: usize = 256;

/// How long the manager waits for a started process to call the dispatcher (SERVICES.md section 3,
/// item 3: measured 30000 ms).
pub const START_TIMEOUT_MS: u64 = 30_000;
/// How long `ControlService` waits for the handler to return.
pub const CONTROL_TIMEOUT_MS: u64 = 30_000;
/// The wait hint the manager shows while a started service has not reported (SERVICES.md section 3,
/// item 1).
pub const START_HINT_MS: u32 = 2000;
