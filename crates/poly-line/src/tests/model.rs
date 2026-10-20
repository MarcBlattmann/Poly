//! Random edits against a simple reference model: a `String` edited by
//! byte offsets, and a `Vec` of history lines. After every edit the editor
//! must hold the model's line and cursor, and the screen (a [`Term`]) must
//! show the prompt and that line with the cursor in the right column.

use super::{PROMPT, Rig, completer};
use crate::{Action, Edit};
use std::string::String;
use std::vec::Vec;

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

struct Model {
    line: String,
    /// In characters.
    cursor: usize,
    history: Vec<String>,
    capacity: usize,
    browsing: Option<usize>,
    draft: String,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Model {
    fn byte(&self, chars: usize) -> usize {
        self.line.char_indices().nth(chars).map_or(self.line.len(), |(b, _)| b)
    }

    fn len(&self) -> usize {
        self.line.chars().count()
    }

    fn before(&self) -> &str {
        &self.line[..self.byte(self.cursor)]
    }

    fn after(&self) -> &str {
        &self.line[self.byte(self.cursor)..]
    }

    fn set_line(&mut self, line: String) {
        self.cursor = line.chars().count();
        self.line = line;
    }

    fn apply(&mut self, e: Edit) -> Option<String> {
        match e {
            Edit::Insert(c) => {
                let at = self.byte(self.cursor);
                self.line.insert(at, c);
                self.cursor += 1;
            }
            Edit::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte(self.cursor);
                self.line.remove(at);
            }
            Edit::Delete if self.cursor < self.len() => {
                let at = self.byte(self.cursor);
                self.line.remove(at);
            }
            Edit::Left => self.cursor = self.cursor.saturating_sub(1),
            Edit::Right => self.cursor = (self.cursor + 1).min(self.len()),
            Edit::Home => self.cursor = 0,
            Edit::End => self.cursor = self.len(),
            Edit::WordLeft => {
                let skip = self.before().chars().rev().take_while(|&c| !is_word(c)).count();
                let word = self.before().chars().rev().skip(skip).take_while(|&c| is_word(c)).count();
                self.cursor -= skip + word;
            }
            Edit::WordRight => {
                let skip = self.after().chars().take_while(|&c| !is_word(c)).count();
                let word = self.after().chars().skip(skip).take_while(|&c| is_word(c)).count();
                self.cursor += skip + word;
            }
            Edit::KillToEnd => {
                let at = self.byte(self.cursor);
                self.line.truncate(at);
            }
            Edit::KillToStart => {
                self.line = String::from(self.after());
                self.cursor = 0;
            }
            Edit::KillWordBefore => {
                let kept = self.before().trim_end_matches(char::is_whitespace);
                let kept = kept.trim_end_matches(|c: char| !c.is_whitespace());
                let removed = self.cursor - kept.chars().count();
                self.line = std::format!("{kept}{}", self.after());
                self.cursor -= removed;
            }
            Edit::HistoryPrev => {
                let back = self.browsing.map_or(0, |b| b + 1);
                if back < self.history.len() {
                    if self.browsing.is_none() {
                        self.draft = core::mem::take(&mut self.line);
                    }
                    self.browsing = Some(back);
                    let line = self.history[self.history.len() - 1 - back].clone();
                    self.set_line(line);
                }
            }
            Edit::HistoryNext => match self.browsing {
                None => {}
                Some(0) => {
                    self.browsing = None;
                    let draft = core::mem::take(&mut self.draft);
                    self.set_line(draft);
                }
                Some(back) => {
                    self.browsing = Some(back - 1);
                    let line = self.history[self.history.len() - back].clone();
                    self.set_line(line);
                }
            },
            Edit::Enter => {
                let line = core::mem::take(&mut self.line);
                if self.capacity > 0 && !line.trim().is_empty() && self.history.last() != Some(&line) {
                    if self.history.len() == self.capacity {
                        self.history.remove(0);
                    }
                    self.history.push(line.clone());
                }
                self.cursor = 0;
                self.browsing = None;
                self.draft.clear();
                return Some(line);
            }
            _ => {}
        }
        None
    }
}

/// Characters of one, two, three and four bytes, word and non-word.
const ALPHABET: [char; 10] = ['a', 'b', ' ', '-', '_', 'ä', '€', 'ж', '😀', '1'];

fn random_edit(rng: &mut Rng) -> Edit {
    use Edit::*;
    const OTHERS: [Edit; 18] = [
        Backspace,
        Delete,
        Left,
        Right,
        WordLeft,
        WordRight,
        Home,
        End,
        KillToEnd,
        KillToStart,
        KillWordBefore,
        HistoryPrev,
        HistoryNext,
        Enter,
        Complete,
        Cancel,
        ClearScreen,
        Redraw,
    ];
    // Mostly typing, so lines get long enough to be interesting.
    if rng.below(2) == 0 { Insert(ALPHABET[rng.below(ALPHABET.len())]) } else { OTHERS[rng.below(OTHERS.len())] }
}

#[test]
fn random_edits_match_the_model() {
    for seed in 1..=40u64 {
        let capacity = (seed % 5) as usize;
        let mut rig = Rig::with_history(capacity);
        let mut model = Model {
            line: String::new(),
            cursor: 0,
            history: Vec::new(),
            capacity,
            browsing: None,
            draft: String::new(),
        };
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let mut complete = completer(&["ab", "abä", "a€", "b-1", "😀"]);
        for step in 0..600 {
            let e = random_edit(&mut rng);
            let got = rig.edit_with(e, &mut complete);
            if e == Edit::Complete {
                // Completion is checked on its own; here it only has to keep
                // the screen right. Take its result over.
                let cursor = rig.ed.cursor();
                model.set_line(rig.ed.line());
                model.cursor = cursor;
                continue;
            }
            if e == Edit::Cancel {
                assert_eq!(got, Action::Cancelled, "seed {seed} step {step}");
                model = Model { line: String::new(), cursor: 0, browsing: None, draft: String::new(), ..model };
                rig.start();
                continue;
            }
            let want = model.apply(e);
            match (got, want) {
                (Action::Done(g), Some(w)) => {
                    assert_eq!(g, w, "seed {seed} step {step}");
                    rig.start();
                }
                (Action::Pending, None) => {}
                (g, w) => panic!("seed {seed} step {step}: {e:?} gave {g:?}, model {w:?}"),
            }
            assert_eq!(rig.ed.line(), model.line, "seed {seed} step {step}: {e:?}");
            assert_eq!(rig.ed.cursor(), model.cursor, "seed {seed} step {step}: {e:?}");
            let history: Vec<&str> = rig.ed.history().iter().collect();
            assert_eq!(history, model.history, "seed {seed} step {step}");
        }
        assert_eq!(rig.ed.prompt(), PROMPT);
    }
}
