//! Package identity (`appmodel.h`, API set `api-ms-win-appmodel-runtime-l1-1-*`), sheet
//! `kernel32.dll/GetCurrentPackageId`: Poly runs no packaged programs, so every process is a plain
//! Win32 one without package identity. The queries answer `APPMODEL_ERROR_NO_PACKAGE` (15700) and
//! leave the length as it was, the application user model id `APPMODEL_ERROR_NO_APPLICATION`
//! (15703), and a NULL length pointer is `ERROR_INVALID_PARAMETER` (87) where the sheet records it.
//! None of them sets the last error (the result is the return value).

use core::ffi::c_void;

/// winerror.h.
const ERROR_INVALID_PARAMETER: i32 = 87;
/// winerror.h `APPMODEL_ERROR_NO_PACKAGE`.
const APPMODEL_ERROR_NO_PACKAGE: i32 = 15700;
/// winerror.h `APPMODEL_ERROR_NO_APPLICATION`.
const APPMODEL_ERROR_NO_APPLICATION: i32 = 15703;

/// The answer of a query whose NULL length pointer the sheet records as an error.
fn no_package_checked(length: *mut u32) -> i32 {
    if length.is_null() { ERROR_INVALID_PARAMETER } else { APPMODEL_ERROR_NO_PACKAGE }
}

/// `GetCurrentPackageId(bufferLength, buffer)`.
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentPackageId(length: *mut u32, _buffer: *mut u8) -> i32 {
    no_package_checked(length)
}

/// `GetCurrentPackageFamilyName(packageFamilyNameLength, packageFamilyName)`.
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentPackageFamilyName(length: *mut u32, _name: *mut u16) -> i32 {
    no_package_checked(length)
}

/// `GetCurrentPackageFullName(packageFullNameLength, packageFullName)`.
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentPackageFullName(_length: *mut u32, _name: *mut u16) -> i32 {
    APPMODEL_ERROR_NO_PACKAGE
}

/// `GetCurrentPackagePath(pathLength, path)`.
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentPackagePath(_length: *mut u32, _path: *mut u16) -> i32 {
    APPMODEL_ERROR_NO_PACKAGE
}

/// `GetPackageFamilyName(hProcess, packageFamilyNameLength, packageFamilyName)`: no process of
/// Poly has a package.
#[unsafe(no_mangle)]
pub extern "system" fn GetPackageFamilyName(_process: *mut c_void, _length: *mut u32, _name: *mut u16) -> i32 {
    APPMODEL_ERROR_NO_PACKAGE
}

/// `GetApplicationUserModelId(hProcess, applicationUserModelIdLength, applicationUserModelId)`.
#[unsafe(no_mangle)]
pub extern "system" fn GetApplicationUserModelId(_process: *mut c_void, _length: *mut u32, _id: *mut u16) -> i32 {
    APPMODEL_ERROR_NO_APPLICATION
}
