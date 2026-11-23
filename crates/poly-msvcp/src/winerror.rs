//! Windows error codes and the system category (`?_Winerror_map@std@@YAHH@Z.yaml`,
//! `?_Winerror_message@std@@YAKKPEADK@Z.yaml`).
//!
//! `std::_Winerror_map(code)` gives the generic (`errno.h`) value a Windows error code
//! corresponds to, or 0 when there is none; `system_category().default_error_condition` is
//! built on it. The pairs are the sheet's table (the Windows codes are `winerror.h`'s, the
//! values the Universal C runtime's `errno.h`, both Windows SDK).
//!
//! `std::_Winerror_message(code, buf, size)` is the system message text of the code
//! (`FormatMessageA` from the system table, which ends the text with CR LF); its result is
//! the length of the text without the trailing CR LF, which stay in the buffer.

/// `(Windows error code, errno value)`, sorted by the code (the sheet's 84 pairs).
pub const MAP: [(i32, i32); 84] = [
    (1, 40),
    (2, 2),
    (3, 2),
    (4, 24),
    (5, 13),
    (6, 22),
    (8, 12),
    (12, 13),
    (14, 12),
    (15, 19),
    (16, 13),
    (17, 18),
    (19, 13),
    (20, 19),
    (21, 11),
    (25, 5),
    (29, 5),
    (30, 5),
    (32, 13),
    (33, 39),
    (39, 28),
    (50, 129),
    (53, 2),
    (55, 19),
    (67, 2),
    (80, 17),
    (82, 13),
    (87, 22),
    (109, 32),
    (110, 5),
    (111, 38),
    (112, 28),
    (121, 138),
    (123, 2),
    (131, 22),
    (142, 16),
    (145, 41),
    (170, 16),
    (183, 17),
    (206, 38),
    (212, 39),
    (258, 138),
    (267, 22),
    (995, 105),
    (998, 13),
    (1011, 5),
    (1012, 5),
    (1013, 5),
    (1237, 11),
    (1460, 138),
    (2401, 16),
    (2404, 16),
    (4393, 22),
    (10004, 4),
    (10009, 9),
    (10013, 13),
    (10014, 14),
    (10022, 22),
    (10024, 24),
    (10035, 140),
    (10036, 112),
    (10037, 103),
    (10038, 128),
    (10039, 109),
    (10040, 115),
    (10041, 136),
    (10042, 123),
    (10043, 135),
    (10045, 130),
    (10047, 102),
    (10048, 100),
    (10049, 101),
    (10050, 116),
    (10051, 118),
    (10052, 117),
    (10053, 106),
    (10054, 108),
    (10055, 119),
    (10056, 113),
    (10057, 126),
    (10060, 138),
    (10061, 107),
    (10063, 38),
    (10065, 110),
];

/// `std::_Winerror_map`: the generic value of a Windows error code, 0 for none.
pub fn map(code: i32) -> i32 {
    MAP.binary_search_by_key(&code, |&(win, _)| win).map_or(0, |i| MAP[i].1)
}

/// `FormatMessageA`'s flags `_Winerror_message` uses: the system table, no inserts
/// (`FORMAT_MESSAGE_FROM_SYSTEM` 0x1000 | `FORMAT_MESSAGE_IGNORE_INSERTS` 0x200, `winbase.h`).
pub const FORMAT_FLAGS: u32 = 0x1000 | 0x200;

/// The language `_Winerror_message` asks for: English (United States),
/// `MAKELANGID(LANG_ENGLISH, SUBLANG_ENGLISH_US)` (`winnt.h`).
pub const LANGUAGE: u32 = 0x0409;

/// The length `_Winerror_message` returns for the `text` `FormatMessageA` wrote: without the
/// trailing CR and LF.
pub fn message_length(text: &[u8]) -> usize {
    let mut n = text.len();
    while n > 0 && matches!(text[n - 1], b'\r' | b'\n') {
        n -= 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_has_84_entries() {
        assert!(MAP.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(MAP.iter().all(|&(_, v)| v != 0));
        let mapped = (-5..70_000).filter(|&c| map(c) != 0).count();
        assert_eq!(mapped, 84);
    }

    #[test]
    fn the_sheets_examples() {
        assert_eq!(map(5), 13);
        assert_eq!(map(2), 2);
        assert_eq!(map(1), 40);
        assert_eq!(map(10061), 107);
        assert_eq!(map(10048), 100);
        assert_eq!(map(10065), 110);
        for none in [0, -1, -5, 1314, 99_999, i32::MAX, i32::MIN] {
            assert_eq!(map(none), 0, "{none}");
        }
    }

    #[test]
    fn the_length_leaves_out_cr_lf() {
        assert_eq!(message_length(b"Access is denied.\r\n"), 17);
        assert_eq!(message_length(b"The operation completed successfully.\r\n"), 37);
        assert_eq!(message_length(b"no line end"), 11);
        assert_eq!(message_length(b"\r\n"), 0);
        assert_eq!(message_length(b""), 0);
    }
}
