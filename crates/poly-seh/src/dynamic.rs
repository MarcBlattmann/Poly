//! Function tables a program registers for code it makes at run time (a JIT compiler's), so that
//! exception dispatch and unwinding find `RUNTIME_FUNCTION` entries for addresses in no image.
//!
//! Learn: "RtlAddFunctionTable" (an array of entries for code at `BaseAddress`; the entries' RVAs
//! are relative to it), "RtlDeleteFunctionTable", "RtlAddGrowableFunctionTable" (a table of
//! `EntryCount` entries the caller may extend up to `MaximumEntryCount`, for the code range
//! `[RangeBase, RangeEnd)`, with the entries' RVAs relative to `RangeBase`; the caller keeps the
//! array, the system refers to it), "RtlGrowFunctionTable" (the table now has `NewEntryCount`
//! entries) and "RtlDeleteGrowableFunctionTable". The entries are sorted, as in an image's
//! exception directory, so a lookup is the same binary search ([`crate::pdata::lookup`]).
//!
//! The registry holds a fixed number of tables (no allocation); the caller (our `ntdll`) keeps
//! it behind a lock. Which table wins when ranges overlap is not documented; the one registered
//! first is searched first here.

/// Tables a process can have registered at once (Poly's limit; a JIT registers one per code
/// region).
pub const MAX_TABLES: usize = 256;

/// One registered table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Table {
    /// The `FunctionTable` pointer the caller passed (what `RtlDeleteFunctionTable` names), and
    /// for a growable table the handle `RtlAddGrowableFunctionTable` returns.
    pub entries: u64,
    /// Entries in use.
    pub count: u32,
    /// Entries the table may grow to (`count` for a table that does not grow).
    pub max: u32,
    /// What the entries' RVAs are relative to.
    pub base: u64,
    /// The code the table covers, `[low, high)`.
    pub low: u64,
    pub high: u64,
    /// Made by `RtlAddGrowableFunctionTable`.
    pub growable: bool,
}

/// The registered tables of a process.
#[derive(Debug)]
pub struct Registry {
    tables: [Option<Table>; MAX_TABLES],
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// Why a table could not be registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddError {
    /// No entries pointer, a range that ends before it begins, or more entries than the maximum.
    Invalid,
    /// Every slot is taken.
    Full,
}

impl Registry {
    pub const fn new() -> Registry {
        Registry { tables: [None; MAX_TABLES] }
    }

    fn add(&mut self, t: Table) -> Result<(), AddError> {
        if t.entries == 0 || t.high < t.low || t.count > t.max {
            return Err(AddError::Invalid);
        }
        let slot = self.tables.iter_mut().find(|s| s.is_none()).ok_or(AddError::Full)?;
        *slot = Some(t);
        Ok(())
    }

    /// `RtlAddFunctionTable(FunctionTable, EntryCount, BaseAddress)`: `low`/`high` are the
    /// lowest begin and highest end of the entries (as addresses), which the caller reads.
    pub fn add_table(&mut self, entries: u64, count: u32, base: u64, low: u64, high: u64) -> Result<(), AddError> {
        self.add(Table { entries, count, max: count, base, low, high, growable: false })
    }

    /// `RtlAddGrowableFunctionTable(DynamicTable*, FunctionTable, EntryCount, MaximumEntryCount,
    /// RangeBase, RangeEnd)`: the handle is the entries pointer.
    pub fn add_growable(&mut self, entries: u64, count: u32, max: u32, low: u64, high: u64) -> Result<u64, AddError> {
        self.add(Table { entries, count, max, base: low, low, high, growable: true })?;
        Ok(entries)
    }

    /// `RtlGrowFunctionTable(DynamicTable, NewEntryCount)`: Learn says the count may only grow up
    /// to the maximum; anything else is ignored. False for an unknown handle.
    pub fn grow(&mut self, handle: u64, count: u32) -> bool {
        match self.tables.iter_mut().flatten().find(|t| t.growable && t.entries == handle) {
            Some(t) => {
                if count >= t.count && count <= t.max {
                    t.count = count;
                }
                true
            }
            None => false,
        }
    }

    /// `RtlDeleteFunctionTable` (`growable` false) or `RtlDeleteGrowableFunctionTable`: whether a
    /// table was removed.
    pub fn delete(&mut self, entries: u64, growable: bool) -> bool {
        for slot in &mut self.tables {
            if slot.is_some_and(|t| t.entries == entries && t.growable == growable) {
                *slot = None;
                return true;
            }
        }
        false
    }

    /// The table whose code range holds `pc`.
    pub fn find(&self, pc: u64) -> Option<Table> {
        self.tables.iter().flatten().find(|t| (t.low..t.high).contains(&pc)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_growable_table_covers_its_range_and_grows_to_its_maximum() {
        let mut r = Registry::new();
        let h = r.add_growable(0x5000, 0, 16, 0x1_0000_0000, 0x1_4000_0000).unwrap();
        assert_eq!(h, 0x5000);
        let t = r.find(0x1_2000_0000).unwrap();
        assert_eq!((t.base, t.count, t.max), (0x1_0000_0000, 0, 16));
        assert_eq!(r.find(0x1_4000_0000), None);
        assert!(r.grow(h, 3));
        assert_eq!(r.find(0x1_0000_0000).unwrap().count, 3);
        // Past the maximum, or shrinking: ignored.
        assert!(r.grow(h, 17));
        assert!(r.grow(h, 2));
        assert_eq!(r.find(0x1_0000_0000).unwrap().count, 3);
        assert!(!r.grow(0x6000, 1));
        // Deleted by its own kind of call only.
        assert!(!r.delete(h, false));
        assert!(r.delete(h, true));
        assert_eq!(r.find(0x1_2000_0000), None);
    }

    #[test]
    fn a_plain_table_is_named_by_its_entries() {
        let mut r = Registry::new();
        r.add_table(0x7000, 4, 0x40_0000, 0x40_1000, 0x40_9000).unwrap();
        assert_eq!(r.find(0x40_1000).unwrap().base, 0x40_0000);
        assert_eq!(r.find(0x40_0fff), None);
        assert_eq!(r.add_table(0, 1, 0, 0, 1), Err(AddError::Invalid));
        assert_eq!(r.add_growable(0x8000, 5, 4, 0, 1), Err(AddError::Invalid));
        assert!(r.delete(0x7000, false));
        assert!(!r.delete(0x7000, false));
    }

    #[test]
    fn the_registry_fills_up() {
        let mut r = Registry::new();
        for i in 0..MAX_TABLES as u64 {
            r.add_table(0x1000 + i, 1, 0, i * 0x100, i * 0x100 + 0x100).unwrap();
        }
        assert_eq!(r.add_table(0x9999, 1, 0, 0, 1), Err(AddError::Full));
    }
}
