//! The functions programs import from `mmdevapi.dll` that Poly does not implement yet (ADR 0155,
//! step 10): each logs `stub: mmdevapi!name` and fails with `E_NOTIMPL`, as the generated stub DLL
//! did. `ord_17` is the export with ordinal 17 (`build.rs`). W2 replaces a name here by defining it
//! elsewhere in the crate and deleting it from the list.

#[cfg(target_arch = "x86_64")]
poly_win32_rt::fail_exports!(
    "mmdevapi",
    hresult,
    [ActivateAudioInterfaceAsync, DriverProc, auxMessage, midMessage, modMessage, ord_17,]
);
