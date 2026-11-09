//! A small binary encoding for the records `user32` and the kernel's
//! win32k exchange ([`crate::proto`]): little-endian integers, booleans as
//! one byte, `Vec<u16>` (UTF-16 text) and `Vec<u32>` with a `u32` count,
//! `Option` with a tag byte. Both sides are built from this crate, so the
//! layout is whatever these impls write; nothing else depends on it.

use alloc::vec::Vec;

/// A reader over received bytes.
#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.bytes.len() < n {
            return None;
        }
        let (a, b) = self.bytes.split_at(n);
        self.bytes = b;
        Some(a)
    }

    /// Whether everything was read.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// A value with a wire form.
pub trait Wire: Sized {
    fn put(&self, out: &mut Vec<u8>);
    fn get(r: &mut Reader<'_>) -> Option<Self>;
}

macro_rules! int {
    ($($t:ty),*) => {$(
        impl Wire for $t {
            fn put(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
            fn get(r: &mut Reader<'_>) -> Option<Self> {
                let b = r.take(core::mem::size_of::<$t>())?;
                Some(<$t>::from_le_bytes(b.try_into().ok()?))
            }
        }
    )*};
}

int!(u8, u16, u32, u64, i32, i64);

impl Wire for bool {
    fn put(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
    fn get(r: &mut Reader<'_>) -> Option<Self> {
        match u8::get(r)? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
}

impl<T: Wire> Wire for Vec<T> {
    fn put(&self, out: &mut Vec<u8>) {
        (self.len() as u32).put(out);
        for v in self {
            v.put(out);
        }
    }
    fn get(r: &mut Reader<'_>) -> Option<Self> {
        let n = u32::get(r)? as usize;
        // Every element takes at least a byte: a count past the bytes left
        // is a broken record, not a reason to allocate.
        if n > r.bytes.len() {
            return None;
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(T::get(r)?);
        }
        Some(v)
    }
}

impl<T: Wire> Wire for Option<T> {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            None => out.push(0),
            Some(v) => {
                out.push(1);
                v.put(out);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Option<Self> {
        match u8::get(r)? {
            0 => Some(None),
            1 => Some(Some(T::get(r)?)),
            _ => None,
        }
    }
}

/// Encode one value.
pub fn encode<T: Wire>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.put(&mut out);
    out
}

/// Decode one value that must use all the bytes.
pub fn decode<T: Wire>(bytes: &[u8]) -> Option<T> {
    let mut r = Reader::new(bytes);
    let v = T::get(&mut r)?;
    r.is_empty().then_some(v)
}

/// A struct with named fields and its wire form (fields in order).
#[macro_export]
macro_rules! wire_struct {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* pub $f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name { $($(#[$fm])* pub $f: $t),* }
        impl $crate::wire::Wire for $name {
            fn put(&self, out: &mut alloc::vec::Vec<u8>) {
                $( $crate::wire::Wire::put(&self.$f, out); )*
            }
            fn get(r: &mut $crate::wire::Reader<'_>) -> Option<Self> {
                Some($name { $($f: <$t as $crate::wire::Wire>::get(r)?),* })
            }
        }
    };
}

/// An enum of struct-like variants, each with a tag byte.
#[macro_export]
macro_rules! wire_enum {
    ($(#[$m:meta])* pub enum $name:ident { $($(#[$vm:meta])* $tag:literal => $var:ident { $($f:ident : $t:ty),* $(,)? }),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub enum $name { $($(#[$vm])* $var { $($f: $t),* }),* }
        impl $crate::wire::Wire for $name {
            #[allow(unused_variables)]
            fn put(&self, out: &mut alloc::vec::Vec<u8>) {
                match self {
                    $($name::$var { $($f),* } => {
                        out.push($tag);
                        $( $crate::wire::Wire::put($f, out); )*
                    })*
                }
            }
            fn get(r: &mut $crate::wire::Reader<'_>) -> Option<Self> {
                match <u8 as $crate::wire::Wire>::get(r)? {
                    $($tag => Some($name::$var { $($f: <$t as $crate::wire::Wire>::get(r)?),* }),)*
                    _ => None,
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    crate::wire_struct! {
        pub struct S { pub a: u32, pub b: Vec<u16>, pub c: Option<i64>, pub d: bool }
    }

    crate::wire_enum! {
        pub enum E {
            1 => One { x: u32 },
            2 => Two { s: S },
            3 => Empty {},
        }
    }

    #[test]
    fn round_trips_and_rejects_garbage() {
        let s = S { a: 7, b: vec![72, 105], c: Some(-5), d: true };
        for e in [E::One { x: 9 }, E::Two { s: s.clone() }, E::Empty {}] {
            assert_eq!(decode::<E>(&encode(&e)), Some(e));
        }
        assert_eq!(decode::<E>(&[9]), None);
        let mut b = encode(&E::Two { s });
        b.push(0);
        assert_eq!(decode::<E>(&b), None, "trailing bytes");
        assert_eq!(decode::<Vec<u16>>(&[0xff, 0xff, 0xff, 0x7f]), None, "a count past the end");
    }
}
