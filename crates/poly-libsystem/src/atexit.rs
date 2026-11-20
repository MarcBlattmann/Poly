//! The functions `exit` calls: `atexit` (C11 7.22.4.2) and `__cxa_atexit`
//! (the Itanium C++ ABI, 3.3.5.3), called in the reverse order of their
//! registration; `__cxa_finalize(dso)` calls (and forgets) those a shared
//! object registered, or all of them for a null `dso`.

/// One registered function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handler {
    /// `atexit(f)`: `f()`.
    Plain(usize),
    /// `__cxa_atexit(f, arg, dso)`: `f(arg)`.
    WithArg { func: usize, arg: usize, dso: usize },
}

impl Handler {
    fn dso(&self) -> usize {
        match *self {
            Handler::Plain(_) => 0,
            Handler::WithArg { dso, .. } => dso,
        }
    }
}

/// How many functions can be registered. C11 asks for at least 32.
pub const MAX: usize = 256;

/// The table of registered functions.
#[derive(Debug)]
pub struct AtExit {
    list: [Option<Handler>; MAX],
    len: usize,
}

impl Default for AtExit {
    fn default() -> Self {
        Self::new()
    }
}

/// The table is full (`atexit` returns non-zero).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full;

impl AtExit {
    pub const fn new() -> Self {
        AtExit { list: [None; MAX], len: 0 }
    }

    pub fn register(&mut self, h: Handler) -> Result<(), Full> {
        if self.len == MAX {
            return Err(Full);
        }
        self.list[self.len] = Some(h);
        self.len += 1;
        Ok(())
    }

    /// The most recently registered function still registered (taken out
    /// of the table): what `exit` calls next. Functions registered while
    /// `exit` runs are called too, before the older ones.
    pub fn pop(&mut self) -> Option<Handler> {
        while self.len > 0 {
            self.len -= 1;
            if let Some(h) = self.list[self.len].take() {
                return Some(h);
            }
        }
        None
    }

    /// The most recent function of `dso` (any, for 0) still registered,
    /// taken out: what `__cxa_finalize(dso)` calls next.
    pub fn pop_for(&mut self, dso: usize) -> Option<Handler> {
        if dso == 0 {
            return self.pop();
        }
        let i = (0..self.len).rev().find(|&i| self.list[i].is_some_and(|h| h.dso() == dso))?;
        self.list[i].take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_order() {
        let mut t = AtExit::new();
        t.register(Handler::Plain(1)).unwrap();
        t.register(Handler::WithArg { func: 2, arg: 20, dso: 0 }).unwrap();
        t.register(Handler::Plain(3)).unwrap();
        assert_eq!(t.pop(), Some(Handler::Plain(3)));
        // Registered while exiting: called next.
        t.register(Handler::Plain(4)).unwrap();
        assert_eq!(t.pop(), Some(Handler::Plain(4)));
        assert_eq!(t.pop(), Some(Handler::WithArg { func: 2, arg: 20, dso: 0 }));
        assert_eq!(t.pop(), Some(Handler::Plain(1)));
        assert_eq!(t.pop(), None);
    }

    #[test]
    fn full_table() {
        let mut t = AtExit::new();
        for i in 0..MAX {
            t.register(Handler::Plain(i)).unwrap();
        }
        assert_eq!(t.register(Handler::Plain(0)), Err(Full));
    }

    #[test]
    fn finalize_one_shared_object() {
        let mut t = AtExit::new();
        t.register(Handler::WithArg { func: 1, arg: 0, dso: 0x1000 }).unwrap();
        t.register(Handler::WithArg { func: 2, arg: 0, dso: 0x2000 }).unwrap();
        t.register(Handler::WithArg { func: 3, arg: 0, dso: 0x1000 }).unwrap();
        assert_eq!(t.pop_for(0x1000), Some(Handler::WithArg { func: 3, arg: 0, dso: 0x1000 }));
        assert_eq!(t.pop_for(0x1000), Some(Handler::WithArg { func: 1, arg: 0, dso: 0x1000 }));
        assert_eq!(t.pop_for(0x1000), None);
        assert_eq!(t.pop(), Some(Handler::WithArg { func: 2, arg: 0, dso: 0x2000 }));
        assert_eq!(t.pop(), None);
    }
}
