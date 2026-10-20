//! The words of a command line: splitting it into arguments, the common
//! prefix of completion candidates, and the closest match for a typo.

use alloc::string::String;
use alloc::vec::Vec;

/// Why a line could not be split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitError {
    /// A quote was opened and never closed.
    UnclosedQuote,
    /// The line ends in a backslash with nothing to escape.
    TrailingBackslash,
}

impl SplitError {
    pub const fn describe(self) -> &'static str {
        match self {
            SplitError::UnclosedQuote => "unclosed quote",
            SplitError::TrailingBackslash => "backslash at the end of the line",
        }
    }
}

/// Split `line` into words at whitespace, like a Unix shell without
/// expansions: `'single quotes'` keep everything inside as it is,
/// `"double quotes"` too except that a backslash escapes the next
/// character, and outside quotes a backslash escapes the next character.
/// Quotes join with what is next to them (`a"b c"d` is one word, `ab cd`),
/// and `""` is an empty word.
pub fn split(line: &str) -> Result<Vec<String>, SplitError> {
    let mut words = Vec::new();
    let mut word = String::new();
    // Whether a word has started; an empty quoted word counts.
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(core::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next().ok_or(SplitError::UnclosedQuote)? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next().ok_or(SplitError::UnclosedQuote)? {
                        '"' => break,
                        '\\' => word.push(chars.next().ok_or(SplitError::UnclosedQuote)?),
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(chars.next().ok_or(SplitError::TrailingBackslash)?);
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

/// The longest prefix all `words` share, by characters (never splitting a
/// UTF-8 sequence). Empty for no words.
pub fn common_prefix<S: AsRef<str>>(words: &[S]) -> &str {
    let Some((first, rest)) = words.split_first() else { return "" };
    let first = first.as_ref();
    let mut end = first.len();
    for w in rest {
        let shared = first.char_indices().zip(w.as_ref().chars()).find(|&((_, a), b)| a != b);
        let here = match shared {
            Some(((i, _), _)) => i,
            // One is a prefix of the other: the shorter one's length.
            None => first.len().min(w.as_ref().len()),
        };
        end = end.min(here);
    }
    &first[..end]
}

/// The edit distance between `a` and `b` in characters: insertions,
/// deletions, substitutions and swaps of two neighbouring characters each
/// count 1 (the optimal string alignment distance), so "hlep" is 1 from
/// "help".
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    // Three rows of the dynamic-programming table: i - 2, i - 1 and i.
    let mut before: Vec<usize> = (0..=b.len()).collect();
    let mut prev = before.clone();
    let mut row = alloc::vec![0; b.len() + 1];
    for i in 1..=a.len() {
        row[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (prev[j] + 1).min(row[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(before[j - 2] + 1);
            }
            row[j] = d;
        }
        core::mem::swap(&mut before, &mut prev);
        core::mem::swap(&mut prev, &mut row);
    }
    prev[b.len()]
}

/// The candidate closest to `word` by [`edit_distance`], if one is close
/// enough to be a likely typo: at most 2 edits, and fewer edits than `word`
/// has characters. The first of equally close candidates wins.
pub fn closest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = word.chars().count().saturating_sub(1).min(2);
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        let d = edit_distance(word, c);
        if d <= limit && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c)
}
