//! Filtering and ranking apps by what was typed.
//!
//! Matching ignores case. Each word of the query must match the app's
//! name, one of its keywords or its program's file name; how well it
//! matches sets its tier (the ordering Alpine Shell's launcher uses, here
//! written down afresh): exact, prefix, the start of a word, anywhere,
//! letters in order with gaps. Matches on keywords or the program name
//! count one tier lower than on the name. Within a tier, shorter names
//! come first, then the alphabet.

use crate::apps::App;
use alloc::string::String;
use alloc::vec::Vec;

/// How well one word matches one text (higher is better; 0 none).
fn tier(word: &str, text: &str) -> u32 {
    if word.is_empty() {
        return 1;
    }
    let text = text.to_lowercase();
    if text == word {
        return 6;
    }
    if text.starts_with(word) {
        return 5;
    }
    let at_word_start =
        text.match_indices(word).any(|(i, _)| text[..i].chars().last().is_some_and(|c| !c.is_alphanumeric()));
    if at_word_start {
        return 4;
    }
    if text.contains(word) {
        return 3;
    }
    let mut chars = text.chars();
    if word.chars().all(|w| chars.any(|t| t == w)) {
        return 2;
    }
    0
}

/// The program's file name (`/usr/bin/foo` → `foo`).
fn program(app: &App) -> &str {
    app.argv.first().map_or("", |p| p.rsplit('/').next().unwrap_or(p))
}

/// The score of `app` for the lower-cased query words; `None` if a word
/// matches nothing.
fn score(app: &App, words: &[String]) -> Option<u32> {
    let mut total = 0;
    for w in words {
        let on_name = tier(w, &app.name);
        let other = app
            .keywords
            .iter()
            .map(|k| tier(w, k))
            .chain(core::iter::once(tier(w, program(app))))
            .max()
            .unwrap_or(0)
            .saturating_sub(1);
        let best = on_name.max(other);
        if best == 0 {
            return None;
        }
        // On a tie, a match on the name wins.
        total += best * 2 + u32::from(on_name == best);
    }
    Some(total)
}

/// The apps matching `query`, best first, at most `limit`. An empty query
/// matches nothing (the launcher shows only its input then, as Alpine's).
pub fn search<'a>(apps: &'a [App], query: &str, limit: usize) -> Vec<&'a App> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u32, &App)> = apps.iter().filter_map(|a| score(a, &words).map(|s| (s, a))).collect();
    hits.sort_by(|(sa, a), (sb, b)| {
        sb.cmp(sa)
            .then(a.name.chars().count().cmp(&b.name.chars().count()))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    hits.into_iter().take(limit).map(|(_, a)| a).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn app(name: &str, keywords: &[&str], prog: &str) -> App {
        App {
            name: name.into(),
            comment: String::new(),
            keywords: keywords.iter().map(|k| (*k).into()).collect(),
            argv: vec![prog.into()],
            id: String::new(),
        }
    }

    fn names(hits: &[&App]) -> Vec<String> {
        hits.iter().map(|a| a.name.clone()).collect()
    }

    #[test]
    fn tiers() {
        assert_eq!(tier("demo", "Demo"), 6);
        assert_eq!(tier("po", "Poly Demo"), 5);
        assert_eq!(tier("de", "Poly Demo"), 4);
        assert_eq!(tier("ly", "Poly Demo"), 3);
        assert_eq!(tier("pdm", "Poly Demo"), 2);
        assert_eq!(tier("x", "Poly Demo"), 0);
    }

    #[test]
    fn ranking() {
        let apps = vec![
            app("Files", &["folder", "explorer"], "/usr/bin/nautilus"),
            app("Firefox", &["web", "browser"], "/usr/bin/firefox"),
            app("Poly Demo", &["demo"], "/bin/wl-demo"),
            app("Settings", &["preferences"], "/usr/bin/settings"),
            app("Terminal", &[], "/usr/bin/foot"),
        ];
        assert!(search(&apps, "", 8).is_empty());
        assert!(search(&apps, "   ", 8).is_empty());
        assert_eq!(names(&search(&apps, "fi", 8)), ["Files", "Firefox"], "both prefixes: the shorter first");
        assert_eq!(names(&search(&apps, "fire", 8)), ["Firefox"]);
        // Keywords and the program's name.
        assert_eq!(names(&search(&apps, "browser", 8)), ["Firefox"]);
        assert_eq!(names(&search(&apps, "foot", 8)), ["Terminal"]);
        // A word start beats a match in the middle.
        assert_eq!(names(&search(&apps, "demo", 8))[0], "Poly Demo");
        // Every word must match, in any order.
        assert_eq!(names(&search(&apps, "demo poly", 8)), ["Poly Demo"]);
        assert!(search(&apps, "poly zzz", 8).is_empty());
        // Letters in order.
        assert_eq!(names(&search(&apps, "trml", 8)), ["Terminal"]);
        assert_eq!(search(&apps, "e", 2).len(), 2, "the limit");
    }
}
