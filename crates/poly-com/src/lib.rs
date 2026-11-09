//! The logic of our `ole32.dll` (ADR 0155), as host-tested code: what
//! Chrome's headless browser needs of COM and the Windows Runtime on a
//! system that has no registered classes.
//!
//! - [`hresult`]: the `HRESULT`s the functions return (`winerror.h`).
//! - [`apartment`]: a thread's COM / Windows Runtime apartment state, shared
//!   by `CoInitializeEx`, `OleInitialize` and `RoInitialize` (Learn:
//!   "CoInitializeEx", "RoInitialize": the first call sets the model, the
//!   same model again returns `S_FALSE`, another one `RPC_E_CHANGED_MODE`,
//!   every successful call is balanced by an uninitialize).
//! - [`guid`]: `GUID`s as text (`CLSIDFromString`, `StringFromCLSID`,
//!   `StringFromGUID2`, `IIDFromString`).
//! - [`hstring`]: `HSTRING`s (`WindowsCreateString` and friends, Learn:
//!   "HSTRING", "Windows Runtime strings").
//!
//! Sources (clean room, CLEAN_ROOM.md): Microsoft Learn's pages for each
//! function and `winerror.h`, `combaseapi.h`, `objbase.h`, `winstring.h`,
//! `roapi.h`, `wtypes.h`.

#![no_std]

extern crate alloc;

pub mod apartment;
pub mod guid;
pub mod hresult;
pub mod hstring;

/// What `ole32.dll` must export, as code (`cargo xtask win32` checks it).
pub const OLE32_EXPORTS: &[&str] = &[
    "CoInitialize",
    "CoInitializeEx",
    "CoUninitialize",
    "CoGetApartmentType",
    "OleInitialize",
    "OleUninitialize",
    "RoInitialize",
    "RoUninitialize",
    "CoTaskMemAlloc",
    "CoTaskMemRealloc",
    "CoTaskMemFree",
    "CoGetClassObject",
    "CoCreateInstance",
    "CoCreateInstanceEx",
    "RoGetActivationFactory",
    "RoActivateInstance",
    "CoCreateFreeThreadedMarshaler",
    "CoInitializeSecurity",
    "CoSetProxyBlanket",
    "CoRegisterMessageFilter",
    "RegisterDragDrop",
    "RevokeDragDrop",
    "CLSIDFromString",
    "IIDFromString",
    "StringFromGUID2",
    "StringFromCLSID",
    "StringFromIID",
    "CoCreateGuid",
    "PropVariantClear",
    "WindowsCreateString",
    "WindowsCreateStringReference",
    "WindowsDeleteString",
    "WindowsDuplicateString",
    "WindowsGetStringRawBuffer",
    "WindowsGetStringLen",
    "WindowsIsStringEmpty",
    "WindowsStringHasEmbeddedNull",
    "WindowsCompareStringOrdinal",
    "WindowsConcatString",
    "WindowsSubstring",
    "WindowsPreallocateStringBuffer",
    "WindowsPromoteStringBuffer",
    "WindowsDeleteStringBuffer",
];
