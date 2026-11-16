//! The one-byte buffer of an unbuffered stream, as `_get_stream_buffer_pointers` shows it (the
//! UCRT keeps it inside the `FILE` record: `_base` points at it, and a read fills it with two
//! bytes: the one the call returns and the one after it, so `_ptr` is one past the first and
//! `_cnt` is 1 while the second is still unread).

/// The two bytes of an unbuffered stream's record buffer and the `_cnt` to show, after a read
/// that returned `last`; `next` is the byte a following read returns (`None` at the end of the
/// data, or where a text-mode read would stop: then the count is 0).
pub fn unbuffered_window(last: u8, next: Option<u8>) -> ([u8; 2], i32) {
    match next {
        Some(n) => ([last, n], 1),
        None => ([last, 0], 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_second_byte_is_the_next_one_to_read() {
        assert_eq!(unbuffered_window(0, Some(1)), ([0, 1], 1));
        assert_eq!(unbuffered_window(7, Some(200)), ([7, 200], 1));
    }

    #[test]
    fn nothing_left_shows_a_zero_count() {
        assert_eq!(unbuffered_window(9, None), ([9, 0], 0));
    }
}
