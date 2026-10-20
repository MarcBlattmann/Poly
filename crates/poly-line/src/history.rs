//! Submitted lines, newest last, for recall with Up / Down.

use alloc::collections::VecDeque;
use alloc::string::String;

/// The last `capacity` submitted lines. Blank lines and a line equal to the
/// newest entry are not added; when full, the oldest entry is dropped.
#[derive(Clone, Debug)]
pub struct History {
    entries: VecDeque<String>,
    capacity: usize,
}

impl History {
    pub fn new(capacity: usize) -> History {
        History { entries: VecDeque::with_capacity(capacity), capacity }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add `line` as the newest entry. False if it was not added: blank,
    /// the same as the newest entry, or a capacity of 0.
    pub fn push(&mut self, line: &str) -> bool {
        if self.capacity == 0 || line.trim().is_empty() || self.entries.back().is_some_and(|e| e == line) {
            return false;
        }
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(line.into());
        true
    }

    /// The entry `back` steps back from the newest: 0 is the newest.
    pub fn get(&self, back: usize) -> Option<&str> {
        let i = self.entries.len().checked_sub(back + 1)?;
        self.entries.get(i).map(String::as_str)
    }

    /// Every entry, oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &str> + ExactSizeIterator {
        self.entries.iter().map(String::as_str)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
