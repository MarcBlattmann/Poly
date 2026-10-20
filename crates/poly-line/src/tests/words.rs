use crate::words::{SplitError, closest, common_prefix, edit_distance, split};
use std::string::String;
use std::vec::Vec;

fn words(line: &str) -> Vec<String> {
    split(line).unwrap()
}

#[test]
fn split_at_whitespace() {
    assert_eq!(words("  test   block::read \t x "), ["test", "block::read", "x"]);
    assert_eq!(words(""), Vec::<String>::new());
    assert_eq!(words("   "), Vec::<String>::new());
    assert_eq!(words("echo ä€"), ["echo", "ä€"]);
}

#[test]
fn quotes_group_and_join() {
    assert_eq!(words(r#"echo "a  b" 'c "d"'"#), ["echo", "a  b", r#"c "d""#]);
    assert_eq!(words(r#"a"b c"d"#), ["ab cd"]);
    assert_eq!(words(r#"x "" ''"#), ["x", "", ""]);
    assert_eq!(words(r#""say \"hi\"" 'no \escape'"#), [r#"say "hi""#, r"no \escape"]);
    assert_eq!(words(r"a\ b \'c"), ["a b", "'c"]);
}

#[test]
fn split_errors() {
    assert_eq!(split("echo 'oops"), Err(SplitError::UnclosedQuote));
    assert_eq!(split(r#"echo "oops"#), Err(SplitError::UnclosedQuote));
    assert_eq!(split(r#"echo "oops\"#), Err(SplitError::UnclosedQuote));
    assert_eq!(split(r"echo \"), Err(SplitError::TrailingBackslash));
    assert!(!SplitError::UnclosedQuote.describe().is_empty());
}

#[test]
fn common_prefix_by_characters() {
    assert_eq!(common_prefix(&["help", "hello", "helium"]), "hel");
    assert_eq!(common_prefix(&["abc"]), "abc");
    assert_eq!(common_prefix(&["abc", "ab"]), "ab");
    assert_eq!(common_prefix(&["x", "y"]), "");
    assert_eq!(common_prefix::<&str>(&[]), "");
    // 'ä' (c3 a4) and 'ö' (c3 b6) share their first byte but not a character.
    assert_eq!(common_prefix(&["aä", "aö"]), "a");
    assert_eq!(common_prefix(&[String::from("€uro"), String::from("€x")]), "€");
}

#[test]
fn edit_distances() {
    assert_eq!(edit_distance("", ""), 0);
    assert_eq!(edit_distance("", "abc"), 3);
    assert_eq!(edit_distance("abc", ""), 3);
    assert_eq!(edit_distance("help", "help"), 0);
    assert_eq!(edit_distance("hlep", "help"), 1, "a swap is one edit");
    assert_eq!(edit_distance("hep", "help"), 1);
    assert_eq!(edit_distance("kitten", "sitting"), 3);
    assert_eq!(edit_distance("ä€", "a€"), 1, "characters, not bytes");
    assert_eq!(edit_distance("ca", "abc"), 3, "the restricted distance does not edit a swapped pair again");
}

#[test]
fn closest_match_for_typos() {
    let commands = ["help", "mem", "ps", "lspci", "disks", "uptime", "reboot"];
    assert_eq!(closest("hlep", commands), Some("help"));
    assert_eq!(closest("lspic", commands), Some("lspci"));
    assert_eq!(closest("uptim", commands), Some("uptime"));
    assert_eq!(closest("disk", commands), Some("disks"));
    assert_eq!(closest("p", commands), None, "a one-letter word has no likely typo");
    assert_eq!(closest("xyzzy", commands), None);
    assert_eq!(closest("mm", commands), Some("mem"));
    assert_eq!(closest("me", ["mem", "me2"]), Some("mem"), "the first of equals wins");
}
