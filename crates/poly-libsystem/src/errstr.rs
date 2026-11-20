//! `strerror`'s texts for the common Darwin errno values (`<sys/errno.h>`
//! numbers; the wording of the system's `intro(2)` manual page). Other
//! numbers get "Unknown error: N", which the libc sheets are to confirm
//! along with the rest of the table.

/// The text of `e`, if it is one this table knows.
pub fn message(e: i32) -> Option<&'static str> {
    Some(match e {
        0 => "Undefined error: 0",
        1 => "Operation not permitted",
        2 => "No such file or directory",
        3 => "No such process",
        4 => "Interrupted system call",
        5 => "Input/output error",
        9 => "Bad file descriptor",
        10 => "No child processes",
        12 => "Cannot allocate memory",
        13 => "Permission denied",
        14 => "Bad address",
        16 => "Resource busy",
        17 => "File exists",
        20 => "Not a directory",
        21 => "Is a directory",
        22 => "Invalid argument",
        24 => "Too many open files",
        25 => "Inappropriate ioctl for device",
        28 => "No space left on device",
        29 => "Illegal seek",
        32 => "Broken pipe",
        33 => "Numerical argument out of domain",
        34 => "Result too large",
        35 => "Resource temporarily unavailable",
        60 => "Operation timed out",
        62 => "Too many levels of symbolic links",
        63 => "File name too long",
        66 => "Directory not empty",
        78 => "Function not implemented",
        84 => "Value too large to be stored in data type",
        92 => "Illegal byte sequence",
        _ => return None,
    })
}

/// Write `strerror(e)` into `buf` (NUL-terminated, cut to fit); the
/// length without the NUL.
pub fn format(e: i32, buf: &mut [u8]) -> usize {
    let mut tmp = [0u8; 40];
    let text: &[u8] = match message(e) {
        Some(m) => m.as_bytes(),
        None => {
            let prefix = b"Unknown error: ";
            tmp[..prefix.len()].copy_from_slice(prefix);
            let mut n = prefix.len();
            let mut digits = [0u8; 12];
            let mut d = 0;
            let mut v = e.unsigned_abs();
            loop {
                digits[d] = b'0' + (v % 10) as u8;
                d += 1;
                v /= 10;
                if v == 0 {
                    break;
                }
            }
            if e < 0 {
                tmp[n] = b'-';
                n += 1;
            }
            for i in (0..d).rev() {
                tmp[n] = digits[i];
                n += 1;
            }
            &tmp[..n]
        }
    };
    if buf.is_empty() {
        return 0;
    }
    let n = text.len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&text[..n]);
    buf[n] = 0;
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown() {
        let mut b = [0u8; 64];
        let n = format(2, &mut b);
        assert_eq!(&b[..n], b"No such file or directory");
        let n = format(999, &mut b);
        assert_eq!(&b[..n], b"Unknown error: 999");
        let n = format(-5, &mut b);
        assert_eq!(&b[..n], b"Unknown error: -5");
        let mut small = [0u8; 4];
        assert_eq!(format(2, &mut small), 3);
        assert_eq!(&small, b"No \0");
    }
}
