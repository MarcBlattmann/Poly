//! Pair kerning: `GPOS` pair adjustment (lookup type 2, also behind type 9
//! extension lookups) of the `kern` feature, and the older `kern` table's
//! format 0 subtables (OpenType specification, "GPOS — Glyph Positioning
//! Table" and "kern — Kerning"; the TrueType Reference Manual describes
//! Apple's `kern` header).
//!
//! Basics only: every `kern` feature of the font is used whatever the
//! script and language, and only the first glyph's x advance adjustment
//! is read (no device tables, no placement changes).

use crate::font::GlyphId;
use crate::reader::{Reader, u16_at, u32_at};

/// The kerning of a pair from a `kern` table: the sum over its horizontal
/// format 0 subtables (`None` if the table cannot be read).
pub(crate) fn kern_pair(kern: &[u8], left: GlyphId, right: GlyphId) -> Option<i16> {
    let key = (u32::from(left.0) << 16) | u32::from(right.0);
    // Microsoft's header: version 0 (u16), nTables (u16). Apple's: version
    // 1.0 as a 16.16 number, nTables (u32), and longer subtable headers.
    let apple = u16_at(kern, 0)? == 1 && u16_at(kern, 2)? == 0;
    let (count, mut at) = if apple { (u32_at(kern, 4)? as usize, 8) } else { (usize::from(u16_at(kern, 2)?), 4) };
    let mut total: i32 = 0;
    for _ in 0..count.min(64) {
        let (len, format, horizontal, cross, minimum, header) = if apple {
            let len = u32_at(kern, at)? as usize;
            let coverage = u16_at(kern, at + 4)?;
            // Apple: 0x8000 vertical, 0x4000 cross-stream, 0x2000 variation.
            (len, coverage & 0xff, coverage & 0x8000 == 0, coverage & 0x4000 != 0, false, 8)
        } else {
            let len = usize::from(u16_at(kern, at + 2)?);
            let coverage = u16_at(kern, at + 4)?;
            (len, coverage >> 8, coverage & 1 != 0, coverage & 4 != 0, coverage & 2 != 0, 6)
        };
        if format == 0 && horizontal && !cross && !minimum {
            let sub = kern.get(at + header..)?;
            let pairs = usize::from(u16_at(sub, 0)?);
            let (mut lo, mut hi) = (0usize, pairs);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let p = 8 + 6 * mid;
                let k = u32_at(sub, p)?;
                match k.cmp(&key) {
                    core::cmp::Ordering::Less => lo = mid + 1,
                    core::cmp::Ordering::Greater => hi = mid,
                    core::cmp::Ordering::Equal => {
                        total += i32::from(u16_at(sub, p + 4)? as i16);
                        break;
                    }
                }
            }
        }
        if len == 0 {
            break;
        }
        at = at.checked_add(len)?;
    }
    Some(total.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16)
}

/// The kerning of a pair from `GPOS`, or `None` if the font has no `kern`
/// feature lookups (so the `kern` table is tried instead).
pub(crate) fn gpos_pair(gpos: &[u8], left: GlyphId, right: GlyphId) -> Option<i16> {
    let feature_list = gpos.get(usize::from(u16_at(gpos, 6)?)..)?;
    let lookup_list = gpos.get(usize::from(u16_at(gpos, 8)?)..)?;
    let lookup_count = usize::from(u16_at(lookup_list, 0)?);
    let mut found = false;
    let mut total: i32 = 0;
    // Each lookup once, in lookup list order, even when several `kern`
    // features (one per script) list it.
    let mut seen = [0u64; 16];
    for f in 0..usize::from(u16_at(feature_list, 0)?) {
        let rec = 2 + 6 * f;
        if feature_list.get(rec..rec + 4)? != b"kern" {
            continue;
        }
        let feature = feature_list.get(usize::from(u16_at(feature_list, rec + 4)?)..)?;
        for i in 0..usize::from(u16_at(feature, 2)?) {
            let l = usize::from(u16_at(feature, 4 + 2 * i)?);
            if l >= lookup_count || l >= 1024 || seen[l / 64] & (1 << (l % 64)) != 0 {
                continue;
            }
            seen[l / 64] |= 1 << (l % 64);
            found = true;
            let lookup = lookup_list.get(usize::from(u16_at(lookup_list, 2 + 2 * l)?)..)?;
            if let Some(v) = lookup_pair(lookup, left, right) {
                total += i32::from(v);
            }
        }
    }
    found.then(|| total.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16)
}

/// The adjustment a type 2 lookup makes for the pair: its first subtable
/// that covers it.
fn lookup_pair(lookup: &[u8], left: GlyphId, right: GlyphId) -> Option<i16> {
    let mut kind = u16_at(lookup, 0)?;
    for s in 0..usize::from(u16_at(lookup, 4)?) {
        let mut sub = lookup.get(usize::from(u16_at(lookup, 6 + 2 * s)?)..)?;
        if kind == 9 {
            // Extension: format 1, the real type, a 32-bit offset.
            kind = u16_at(sub, 2)?;
            sub = sub.get(u32_at(sub, 4)? as usize..)?;
        }
        if kind != 2 {
            return None;
        }
        if let Some(v) = pair_subtable(sub, left, right) {
            return Some(v);
        }
    }
    None
}

/// Bytes of a value record with this format (two per field present).
fn value_size(format: u16) -> usize {
    2 * (format & 0xff).count_ones() as usize
}

/// The x advance field of a value record (0 if the format has none).
fn x_advance(record: &[u8], format: u16) -> Option<i16> {
    if format & 0x0004 == 0 {
        return Some(0);
    }
    u16_at(record, value_size(format & 0x0003)).map(|v| v as i16)
}

fn pair_subtable(sub: &[u8], left: GlyphId, right: GlyphId) -> Option<i16> {
    let format = u16_at(sub, 0)?;
    let coverage = sub.get(usize::from(u16_at(sub, 2)?)..)?;
    let (vf1, vf2) = (u16_at(sub, 4)?, u16_at(sub, 6)?);
    let cov = coverage_index(coverage, left)?;
    match format {
        1 => {
            let set = sub.get(usize::from(u16_at(sub, 10 + 2 * usize::from(cov))?)..)?;
            let rec = 2 + value_size(vf1) + value_size(vf2);
            let (mut lo, mut hi) = (0usize, usize::from(u16_at(set, 0)?));
            while lo < hi {
                let mid = (lo + hi) / 2;
                let at = 2 + rec * mid;
                let second = u16_at(set, at)?;
                match second.cmp(&right.0) {
                    core::cmp::Ordering::Less => lo = mid + 1,
                    core::cmp::Ordering::Greater => hi = mid,
                    core::cmp::Ordering::Equal => return x_advance(set.get(at + 2..)?, vf1),
                }
            }
            None
        }
        2 => {
            let class1 = class_of(sub.get(usize::from(u16_at(sub, 8)?)..)?, left);
            let class2 = class_of(sub.get(usize::from(u16_at(sub, 10)?)..)?, right);
            let (count1, count2) = (u16_at(sub, 12)?, u16_at(sub, 14)?);
            if class1 >= count1 || class2 >= count2 {
                return Some(0);
            }
            let rec = value_size(vf1) + value_size(vf2);
            let at = 16 + rec * (usize::from(class1) * usize::from(count2) + usize::from(class2));
            x_advance(sub.get(at..)?, vf1)
        }
        _ => None,
    }
}

/// A glyph's index in a coverage table.
pub(crate) fn coverage_index(cov: &[u8], g: GlyphId) -> Option<u16> {
    match u16_at(cov, 0)? {
        1 => {
            let (mut lo, mut hi) = (0usize, usize::from(u16_at(cov, 2)?));
            while lo < hi {
                let mid = (lo + hi) / 2;
                let v = u16_at(cov, 4 + 2 * mid)?;
                match v.cmp(&g.0) {
                    core::cmp::Ordering::Less => lo = mid + 1,
                    core::cmp::Ordering::Greater => hi = mid,
                    core::cmp::Ordering::Equal => return u16::try_from(mid).ok(),
                }
            }
            None
        }
        2 => {
            let (mut lo, mut hi) = (0usize, usize::from(u16_at(cov, 2)?));
            while lo < hi {
                let mid = (lo + hi) / 2;
                let at = 4 + 6 * mid;
                let (start, end) = (u16_at(cov, at)?, u16_at(cov, at + 2)?);
                if g.0 < start {
                    hi = mid;
                } else if g.0 > end {
                    lo = mid + 1;
                } else {
                    return u16_at(cov, at + 4)?.checked_add(g.0 - start);
                }
            }
            None
        }
        _ => None,
    }
}

/// A glyph's class in a class definition table (0 if not listed).
fn class_of(def: &[u8], g: GlyphId) -> u16 {
    let class = || -> Option<u16> {
        let mut r = Reader::new(def);
        match r.u16()? {
            1 => {
                let (start, count) = (r.u16()?, r.u16()?);
                let i = g.0.checked_sub(start).filter(|&i| i < count)?;
                u16_at(def, 6 + 2 * usize::from(i))
            }
            2 => {
                let n = usize::from(r.u16()?);
                let (mut lo, mut hi) = (0usize, n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let at = 4 + 6 * mid;
                    let (start, end) = (u16_at(def, at)?, u16_at(def, at + 2)?);
                    if g.0 < start {
                        hi = mid;
                    } else if g.0 > end {
                        lo = mid + 1;
                    } else {
                        return u16_at(def, at + 4);
                    }
                }
                None
            }
            _ => None,
        }
    };
    class().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn be(v: &[u16]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }

    /// A GPOS with one `kern` feature and two lookups: a format 1 pair
    /// subtable (glyph 5 then 7: -30; value format XPlacement|XAdvance), and
    /// an extension lookup holding a format 2 class subtable (class 1 =
    /// glyphs 10-11, then class 1 = glyph 20: -55).
    fn gpos() -> Vec<u8> {
        // Format 1 subtable: format, coverage@12, vf1=0x0005, vf2=0,
        // pairSetCount=1, pairSet@18; coverage fmt1 [5]; pairSet: 1 record
        // (second 7, XPlacement 3, XAdvance -30).
        let mut pp1 = be(&[1, 12, 0x0005, 0, 1, 18]);
        pp1.extend(be(&[1, 1, 5]));
        pp1.extend(be(&[1, 7, 3, (-30i16) as u16]));
        // Format 2 subtable: format, coverage@24, vf1=4, vf2=0, cd1@34, cd2@44,
        // class1Count=2, class2Count=2, then 4 records of one i16.
        let mut pp2 = be(&[2, 24, 4, 0, 34, 44, 2, 2]);
        pp2.extend(be(&[0, 0, 0, (-55i16) as u16]));
        assert_eq!(pp2.len(), 24);
        pp2.extend(be(&[2, 1, 10, 11, 0])); // coverage fmt 2: 10..11
        pp2.extend(be(&[2, 1, 10, 11, 1])); // classdef1 fmt 2: 10..11 -> 1
        pp2.extend(be(&[1, 20, 1, 1])); // classdef2 fmt 1: start 20, 1 glyph, class 1
        // Lookup 0: type 2, flag 0, 1 subtable at 8.
        let mut l0 = be(&[2, 0, 1, 8]);
        l0.extend(&pp1);
        // Lookup 1: type 9, 1 subtable at 8: extension fmt 1, type 2, offset 8.
        let mut l1 = be(&[9, 0, 1, 8]);
        l1.extend(be(&[1, 2, 0, 8]));
        l1.extend(&pp2);
        // Lookup list: count 2, offsets.
        let mut ll = be(&[2, 6, (6 + l0.len()) as u16]);
        ll.extend(&l0);
        ll.extend(&l1);
        // Feature list: 1 record 'kern' at 8; feature: params 0, 2 lookups.
        let mut fl = be(&[1]);
        fl.extend(b"kern");
        fl.extend(be(&[8, 0, 2, 0, 1]));
        // Header: 1.0, script list (empty) at 10, features at 12, lookups.
        let mut g = be(&[1, 0, 10, 12, (12 + fl.len()) as u16]);
        g.extend(be(&[0]));
        g.extend(&fl);
        g.extend(&ll);
        g
    }

    #[test]
    fn gpos_pair_adjustments() {
        let g = gpos();
        assert_eq!(gpos_pair(&g, GlyphId(5), GlyphId(7)), Some(-30));
        assert_eq!(gpos_pair(&g, GlyphId(5), GlyphId(8)), Some(0));
        assert_eq!(gpos_pair(&g, GlyphId(10), GlyphId(20)), Some(-55));
        assert_eq!(gpos_pair(&g, GlyphId(11), GlyphId(20)), Some(-55));
        assert_eq!(gpos_pair(&g, GlyphId(11), GlyphId(21)), Some(0));
        assert_eq!(gpos_pair(&g, GlyphId(1), GlyphId(2)), Some(0));
        for len in 0..g.len() {
            let _ = gpos_pair(&g[..len], GlyphId(10), GlyphId(20));
        }
    }

    #[test]
    fn kern_table_pairs() {
        // Microsoft header, one format 0 horizontal subtable, two pairs.
        let mut k = be(&[0, 1, 0, 6 + 8 + 12, 0x0001, 2, 12, 1, 0]);
        k.extend(be(&[1, 2, (-40i16) as u16, 3, 1, 25]));
        assert_eq!(kern_pair(&k, GlyphId(1), GlyphId(2)), Some(-40));
        assert_eq!(kern_pair(&k, GlyphId(3), GlyphId(1)), Some(25));
        assert_eq!(kern_pair(&k, GlyphId(2), GlyphId(1)), Some(0));
        // Apple header: version 1.0, 1 table, length, coverage (format 0).
        let mut a = be(&[1, 0, 0, 1, 0, 8 + 8 + 6, 0x0000, 0, 1, 6, 0, 0]);
        a.extend(be(&[4, 5, (-7i16) as u16]));
        assert_eq!(kern_pair(&a, GlyphId(4), GlyphId(5)), Some(-7));
    }
}
