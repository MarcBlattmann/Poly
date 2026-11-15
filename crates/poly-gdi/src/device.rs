//! Which devices `CreateDCW` / `CreateICW` open, and what `GetICMProfileW`
//! answers (CreateDCW.yaml, GetICMProfileW.yaml): the rules without a DC.

use alloc::vec::Vec;

/// `ERROR_INVALID_PRINTER_NAME` (winerror.h).
pub const ERROR_INVALID_PRINTER_NAME: u32 = 1801;

fn eq_ignore_case(a: &[u16], b: &str) -> bool {
    a.len() == b.len() && a.iter().zip(b.bytes()).all(|(&x, y)| x < 128 && (x as u8).eq_ignore_ascii_case(&y))
}

/// Whether `s` is a display device path (two backslashes, a dot, a
/// backslash and a name: `\\.\DISPLAY1`).
fn is_device_path(s: &[u16]) -> bool {
    s.len() > 4 && s[..4] == [b'\\' as u16, b'\\' as u16, b'.' as u16, b'\\' as u16]
}

/// `CreateDCW(driver, device, ...)` of the display: `Ok` when a DC of the
/// screen is made; `Err(None)` for NULL with the last error left as it
/// was, `Err(Some(code))` for NULL with that error.
///
/// - a device name that is a display device path gives the DC (with the
///   driver NULL or `DISPLAY`);
/// - any other device name: `ERROR_INVALID_PRINTER_NAME`;
/// - no device name: the driver must be `DISPLAY` (any case), anything
///   else (empty, made up, NULL) fails with the error untouched.
pub fn open_display(driver: Option<&[u16]>, device: Option<&[u16]>) -> Result<(), Option<u32>> {
    let display = driver.is_some_and(|d| eq_ignore_case(d, "DISPLAY"));
    match device {
        Some(dev) if is_device_path(dev) => {
            if driver.is_none() || display {
                Ok(())
            } else {
                Err(Some(ERROR_INVALID_PRINTER_NAME))
            }
        }
        Some(_) => Err(Some(ERROR_INVALID_PRINTER_NAME)),
        None if display => Ok(()),
        None => Err(None),
    }
}

/// The colour profile `GetICMProfileW` names for every DC of the display
/// (the system's default sRGB profile).
pub const ICM_PROFILE: &str = "C:\\Windows\\System32\\spool\\drivers\\color\\sRGB Color Space Profile.icm";

/// What `GetICMProfileW(dc, &size, buffer)` does for a valid DC.
#[derive(Debug, PartialEq, Eq)]
pub enum IcmAnswer {
    /// Success: the path and its terminating NUL (written to the buffer),
    /// `*size` becomes its length in characters.
    Path(Vec<u16>),
    /// `ERROR_INSUFFICIENT_BUFFER` (122): the buffer is not touched and
    /// `*size` is the size needed.
    TooSmall(u32),
}

/// `GetICMProfileW` for a DC of the display: `size_in` is `*size` on entry.
pub fn icm_profile(size_in: u32, has_buffer: bool) -> IcmAnswer {
    let mut path: Vec<u16> = ICM_PROFILE.encode_utf16().collect();
    path.push(0);
    let need = path.len() as u32;
    if !has_buffer || size_in < need { IcmAnswer::TooSmall(need) } else { IcmAnswer::Path(path) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn displays_and_failures() {
        let d = w("DISPLAY");
        assert_eq!(open_display(Some(&d), None), Ok(()));
        assert_eq!(open_display(Some(&w("display")), None), Ok(()));
        // A made-up device name that is no device path.
        assert_eq!(open_display(Some(&w("NoSuchDevice")), Some(&w("NoSuchDevice"))), Err(Some(1801)));
        assert_eq!(open_display(Some(&d), Some(&w("\\\\.\\NoSuchDisplay"))), Ok(()));
        assert_eq!(open_display(None, Some(&w("\\\\.\\DISPLAY1"))), Ok(()));
        // No device: only DISPLAY opens, the others leave the error alone.
        assert_eq!(open_display(Some(&w("")), None), Err(None));
        assert_eq!(open_display(Some(&w("NoSuchDriver")), None), Err(None));
        assert_eq!(open_display(None, None), Err(None));
    }

    #[test]
    fn icm_sizes() {
        let IcmAnswer::Path(p) = icm_profile(1024, true) else { panic!() };
        assert_eq!(p.len() as u32, ICM_PROFILE.len() as u32 + 1);
        assert_eq!(icm_profile(0, true), IcmAnswer::TooSmall(p.len() as u32));
        assert_eq!(icm_profile(p.len() as u32 - 1, true), IcmAnswer::TooSmall(p.len() as u32));
        assert!(matches!(icm_profile(p.len() as u32, true), IcmAnswer::Path(_)));
        assert_eq!(icm_profile(1024, false), IcmAnswer::TooSmall(p.len() as u32));
    }
}
