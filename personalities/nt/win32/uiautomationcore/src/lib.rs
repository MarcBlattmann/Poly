//! Our `uiautomationcore.dll` (ADR 0155, step 9): the provider-side entry points of UI
//! Automation that Chrome's browser delay-loads. Poly has no UI Automation clients (no screen
//! reader, no test tool that speaks it), so: nobody is listening, no host provider exists for a
//! window, raising an event reaches no one (which is success: Learn's provider functions
//! succeed when there are no listeners), and a `WM_GETOBJECT` reply has nothing to marshal to.
//! The DLL has to exist: a delay load that finds no DLL ends the browser.
//!
//! Learn (`uiautomationcoreapi.h`): "UiaReturnRawElementProvider", "UiaHostProviderFromHwnd",
//! "UiaDisconnectProvider", "UiaDisconnectAllProviders", "UiaRaiseAutomationEvent",
//! "UiaRaiseAutomationPropertyChangedEvent", "UiaRaiseStructureChangedEvent",
//! "UiaGetReservedNotSupportedValue", "UiaGetReservedMixedAttributeValue",
//! "UiaClientsAreListening". Not observed on Windows.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_win32_rt as _;

/// `S_OK`, `E_NOTIMPL`, `E_INVALIDARG` (`winerror.h`).
const S_OK: u32 = 0;
const E_NOTIMPL: u32 = 0x8000_4001;
const E_INVALIDARG: u32 = 0x8007_0057;

/// `UiaReturnRawElementProvider(hwnd, wParam, lParam, el)`: the `LRESULT` for `WM_GETOBJECT`.
/// Learn: "Returns the key for the client process to connect to the server process through UI
/// Automation"; with no client there is no connection, and 0 says the message was not handled.
#[unsafe(no_mangle)]
pub extern "system" fn UiaReturnRawElementProvider(
    _hwnd: usize,
    _wparam: usize,
    _lparam: isize,
    _el: *mut u8,
) -> isize {
    0
}

/// `UiaHostProviderFromHwnd(hwnd, ppProvider)`: the host provider of a window. Poly has none.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UiaHostProviderFromHwnd(_hwnd: usize, provider: *mut *mut u8) -> u32 {
    if provider.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { provider.write(core::ptr::null_mut()) };
    E_NOTIMPL
}

/// `UiaDisconnectProvider(pProvider)`: no client holds the provider, so there is nothing to
/// release.
#[unsafe(no_mangle)]
pub extern "system" fn UiaDisconnectProvider(provider: *mut u8) -> u32 {
    if provider.is_null() { E_INVALIDARG } else { S_OK }
}

/// `UiaDisconnectAllProviders()`.
#[unsafe(no_mangle)]
pub extern "system" fn UiaDisconnectAllProviders() -> u32 {
    S_OK
}

/// `UiaClientsAreListening()`: FALSE.
#[unsafe(no_mangle)]
pub extern "system" fn UiaClientsAreListening() -> i32 {
    0
}

/// `UiaRaiseAutomationEvent(pProvider, id)`: no client listens; the event reaches no one.
#[unsafe(no_mangle)]
pub extern "system" fn UiaRaiseAutomationEvent(_provider: *mut u8, _id: i32) -> u32 {
    S_OK
}

/// `UiaRaiseAutomationPropertyChangedEvent(pProvider, id, oldValue, newValue)`. The two
/// `VARIANT`s are passed by value: on x64 that is a pointer to each.
#[unsafe(no_mangle)]
pub extern "system" fn UiaRaiseAutomationPropertyChangedEvent(
    _provider: *mut u8,
    _id: i32,
    _old: *const u8,
    _new: *const u8,
) -> u32 {
    S_OK
}

/// `UiaRaiseStructureChangedEvent(pProvider, structureChangeType, pRuntimeId, cRuntimeIdLen)`.
#[unsafe(no_mangle)]
pub extern "system" fn UiaRaiseStructureChangedEvent(
    _provider: *mut u8,
    _change: i32,
    _runtime_id: *const i32,
    _len: i32,
) -> u32 {
    S_OK
}

/// Clear an out-pointer: the reserved value objects do not exist on Poly.
///
/// # Safety
/// `value` is null or the caller's `IUnknown*` slot.
unsafe fn no_reserved_value(value: *mut *mut u8) -> u32 {
    if value.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { value.write(core::ptr::null_mut()) };
    E_NOTIMPL
}

/// `UiaGetReservedNotSupportedValue(punkNotSupportedValue)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UiaGetReservedNotSupportedValue(value: *mut *mut u8) -> u32 {
    // SAFETY: the caller's out-pointer.
    unsafe { no_reserved_value(value) }
}

/// `UiaGetReservedMixedAttributeValue(punkMixedAttributeValue)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UiaGetReservedMixedAttributeValue(value: *mut *mut u8) -> u32 {
    // SAFETY: the caller's out-pointer.
    unsafe { no_reserved_value(value) }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn uiautomationcore_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
