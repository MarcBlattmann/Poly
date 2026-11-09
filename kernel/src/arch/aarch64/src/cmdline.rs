//! The kernel command line (`/chosen/bootargs`, QEMU's `-append`): words
//! separated by whitespace, like the x86 kernel's `poly.*` switches.

/// Whether `flag` is one of the words of `cmdline`, exactly.
pub fn has_flag(cmdline: &str, flag: &str) -> bool {
    cmdline.split_whitespace().any(|w| w == flag)
}

#[cfg(test)]
mod tests {
    use super::has_flag;

    #[test]
    fn matches_whole_words() {
        assert!(has_flag("poly.a poly.selftest.panic", "poly.selftest.panic"));
        assert!(has_flag("  poly.selftest.panic\t", "poly.selftest.panic"));
        assert!(!has_flag("poly.selftest.panic2", "poly.selftest.panic"));
        assert!(!has_flag("x.poly.selftest.panic", "poly.selftest.panic"));
        assert!(!has_flag("", "poly.selftest.panic"));
    }
}
