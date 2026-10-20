//! The resource tree (spec "The .rsrc Section"): directory tables of
//! entries keyed by name or ID, normally three levels deep (type, name,
//! language), ending in data entries. Offsets inside the tree count from
//! the start of the resource directory; data entries hold RVAs.

use crate::headers::Pe;
use crate::read::{bytes, u16_at, u32_at};
use crate::{Error, MAX_RESOURCE_VISITS, dir};

/// Size of a directory table header and of one entry.
const TABLE: usize = 16;
const ENTRY: usize = 8;
/// Size of a data entry.
const DATA_ENTRY: usize = 16;
/// The high bit marks a name (in the name field) or a subdirectory (in the
/// offset field).
const HIGH: u32 = 0x8000_0000;
/// Deepest tree [`ResourceDirectory::walk`] follows.
const MAX_DEPTH: usize = 8;

/// A UTF-16LE string from the resource tree (a length-prefixed "Resource
/// Directory String").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utf16Str<'a> {
    bytes: &'a [u8],
}

impl<'a> Utf16Str<'a> {
    /// Number of UTF-16 code units.
    pub fn len(&self) -> usize {
        self.bytes.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn units(&self) -> impl Iterator<Item = u16> + 'a {
        self.bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]))
    }

    /// Compares with an ASCII string, ignoring ASCII case (how resource
    /// names are matched: resource compilers store them upper-cased).
    pub fn eq_ignore_ascii_case(&self, s: &str) -> bool {
        self.len() == s.len()
            && self.units().zip(s.bytes()).all(|(u, c)| u < 0x80 && (u as u8).eq_ignore_ascii_case(&c))
    }
}

/// An entry's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceName<'a> {
    Id(u32),
    Name(Utf16Str<'a>),
}

/// What to look an entry up by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKey<'k> {
    Id(u32),
    /// Matched ignoring ASCII case.
    Name(&'k str),
}

impl ResourceKey<'_> {
    fn matches(&self, name: &ResourceName<'_>) -> bool {
        match (self, name) {
            (ResourceKey::Id(a), ResourceName::Id(b)) => a == b,
            (ResourceKey::Name(a), ResourceName::Name(b)) => b.eq_ignore_ascii_case(a),
            _ => false,
        }
    }
}

/// A leaf: where the resource's bytes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceData {
    /// An RVA (not an offset in the tree).
    pub data_rva: u32,
    pub size: u32,
    pub codepage: u32,
    pub reserved: u32,
}

/// One entry of a directory table.
#[derive(Clone, Copy, Debug)]
pub struct ResourceEntry<'a> {
    pub name: ResourceName<'a>,
    tree: &'a [u8],
    target: u32,
}

impl<'a> ResourceEntry<'a> {
    /// The subdirectory this entry leads to, if it is not a leaf.
    pub fn directory(&self) -> Result<Option<ResourceDirectory<'a>>, Error> {
        if self.target & HIGH == 0 {
            return Ok(None);
        }
        ResourceDirectory::at(self.tree, self.target & !HIGH).map(Some)
    }

    /// The data entry this leaf points at, if it is a leaf.
    pub fn data(&self) -> Result<Option<ResourceData>, Error> {
        if self.target & HIGH != 0 {
            return Ok(None);
        }
        let b = bytes(self.tree, self.target as usize, DATA_ENTRY).ok_or(Error::Truncated)?;
        let u = |o| u32_at(b, o).unwrap_or(0);
        Ok(Some(ResourceData { data_rva: u(0), size: u(4), codepage: u(8), reserved: u(12) }))
    }
}

/// A resource directory table.
#[derive(Clone, Copy, Debug)]
pub struct ResourceDirectory<'a> {
    pub characteristics: u32,
    pub time_date_stamp: u32,
    pub major_version: u16,
    pub minor_version: u16,
    pub number_of_named_entries: u16,
    pub number_of_id_entries: u16,
    tree: &'a [u8],
    offset: u32,
}

impl<'a> ResourceDirectory<'a> {
    fn at(tree: &'a [u8], offset: u32) -> Result<Self, Error> {
        let b = bytes(tree, offset as usize, TABLE).ok_or(Error::Truncated)?;
        let u16 = |o| u16_at(b, o).unwrap_or(0);
        let u32 = |o| u32_at(b, o).unwrap_or(0);
        Ok(ResourceDirectory {
            characteristics: u32(0),
            time_date_stamp: u32(4),
            major_version: u16(8),
            minor_version: u16(10),
            number_of_named_entries: u16(12),
            number_of_id_entries: u16(14),
            tree,
            offset,
        })
    }

    pub fn len(&self) -> usize {
        usize::from(self.number_of_named_entries) + usize::from(self.number_of_id_entries)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Entry `index`: named entries come first, then ID entries.
    pub fn entry(&self, index: usize) -> Result<ResourceEntry<'a>, Error> {
        if index >= self.len() {
            return Err(Error::Truncated);
        }
        let at = self.offset as usize + TABLE + index * ENTRY;
        let b = bytes(self.tree, at, ENTRY).ok_or(Error::Truncated)?;
        let (key, target) = (u32_at(b, 0).unwrap_or(0), u32_at(b, 4).unwrap_or(0));
        let name = if key & HIGH != 0 {
            let off = (key & !HIGH) as usize;
            let units = u16_at(self.tree, off).ok_or(Error::Truncated)?;
            let s = bytes(self.tree, off + 2, usize::from(units) * 2).ok_or(Error::Truncated)?;
            ResourceName::Name(Utf16Str { bytes: s })
        } else {
            ResourceName::Id(key)
        };
        Ok(ResourceEntry { name, tree: self.tree, target })
    }

    pub fn entries(&self) -> ResourceEntries<'a> {
        ResourceEntries { dir: *self, next: 0 }
    }

    /// The first entry matching `key`.
    pub fn find(&self, key: ResourceKey<'_>) -> Result<Option<ResourceEntry<'a>>, Error> {
        for e in self.entries() {
            let e = e?;
            if key.matches(&e.name) {
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    /// Calls `f` with the path of names and the data entry of every leaf
    /// under this directory, depth first. Stops at [`MAX_RESOURCE_VISITS`]
    /// entries ([`Error::TooMany`]) and refuses trees deeper than 8.
    pub fn walk(&self, mut f: impl FnMut(&[ResourceName<'a>], ResourceData)) -> Result<(), Error> {
        let mut path = [ResourceName::Id(0); MAX_DEPTH];
        let mut visits = 0usize;
        self.walk_inner(&mut path, 0, &mut visits, &mut f)
    }

    fn walk_inner(
        &self,
        path: &mut [ResourceName<'a>; MAX_DEPTH],
        depth: usize,
        visits: &mut usize,
        f: &mut impl FnMut(&[ResourceName<'a>], ResourceData),
    ) -> Result<(), Error> {
        for e in self.entries() {
            let e = e?;
            *visits += 1;
            if *visits > MAX_RESOURCE_VISITS {
                return Err(Error::TooMany);
            }
            path[depth] = e.name;
            if let Some(d) = e.data()? {
                f(&path[..=depth], d);
            } else if let Some(sub) = e.directory()? {
                if depth + 1 >= MAX_DEPTH {
                    return Err(Error::Malformed("resource tree too deep"));
                }
                sub.walk_inner(path, depth + 1, visits, f)?;
            }
        }
        Ok(())
    }
}

/// Iterator over a directory table's entries. It stops after the first error.
#[derive(Clone, Debug)]
pub struct ResourceEntries<'a> {
    dir: ResourceDirectory<'a>,
    next: usize,
}

impl<'a> Iterator for ResourceEntries<'a> {
    type Item = Result<ResourceEntry<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.dir.len() {
            return None;
        }
        let r = self.dir.entry(self.next);
        self.next = if r.is_ok() { self.next + 1 } else { usize::MAX };
        Some(r)
    }
}

impl<'a> Pe<'a> {
    /// The root of the resource tree, if the image has one.
    pub fn resources(&self) -> Result<Option<ResourceDirectory<'a>>, Error> {
        let Some(d) = self.info().directory(dir::RESOURCE) else { return Ok(None) };
        // Offsets in the tree may point anywhere in the rest of the section.
        let tree = self.view.rest(d.virtual_address)?;
        ResourceDirectory::at(tree, 0).map(Some)
    }

    /// Looks up type / name / language. With `language` `None`, the first
    /// language present is taken.
    pub fn find_resource(
        &self,
        kind: ResourceKey<'_>,
        name: ResourceKey<'_>,
        language: Option<u32>,
    ) -> Result<Option<ResourceData>, Error> {
        let Some(root) = self.resources()? else { return Ok(None) };
        let Some(by_kind) = root.find(kind)?.map(|e| e.directory()).transpose()?.flatten() else { return Ok(None) };
        let Some(by_name) = by_kind.find(name)?.map(|e| e.directory()).transpose()?.flatten() else { return Ok(None) };
        let leaf = match language {
            Some(l) => by_name.find(ResourceKey::Id(l))?,
            None => by_name.entries().next().transpose()?,
        };
        match leaf {
            Some(e) => e.data(),
            None => Ok(None),
        }
    }

    /// The bytes a data entry describes.
    pub fn resource_bytes(&self, data: &ResourceData) -> Result<&'a [u8], Error> {
        if data.size == 0 {
            return Ok(&[]);
        }
        self.view.slice(data.data_rva, data.size as usize)
    }
}
