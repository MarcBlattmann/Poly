//! A table of values addressed by index and generation, so a stale id never
//! reaches a value that reused its slot.

use alloc::vec::Vec;

#[derive(Debug)]
struct Entry<T> {
    generation: u32,
    value: Option<T>,
}

#[derive(Debug)]
pub(crate) struct Slab<T> {
    slots: Vec<Entry<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> Slab<T> {
    pub(crate) const fn new() -> Self {
        Slab { slots: Vec::new(), free: Vec::new(), len: 0 }
    }

    pub(crate) fn insert(&mut self, value: T) -> (u32, u32) {
        self.len += 1;
        if let Some(index) = self.free.pop() {
            let e = &mut self.slots[index as usize];
            e.value = Some(value);
            (index, e.generation)
        } else {
            let index = u32::try_from(self.slots.len()).expect("slab index overflow");
            self.slots.push(Entry { generation: 0, value: Some(value) });
            (index, 0)
        }
    }

    pub(crate) fn get(&self, index: u32, generation: u32) -> Option<&T> {
        let e = self.slots.get(index as usize)?;
        if e.generation == generation { e.value.as_ref() } else { None }
    }

    pub(crate) fn get_mut(&mut self, index: u32, generation: u32) -> Option<&mut T> {
        let e = self.slots.get_mut(index as usize)?;
        if e.generation == generation { e.value.as_mut() } else { None }
    }

    pub(crate) fn remove(&mut self, index: u32, generation: u32) -> Option<T> {
        let e = self.slots.get_mut(index as usize)?;
        if e.generation != generation {
            return None;
        }
        let v = e.value.take()?;
        e.generation = e.generation.wrapping_add(1);
        self.free.push(index);
        self.len -= 1;
        Some(v)
    }

    /// Live entries in index order: `(index, generation, value)`.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (u32, u32, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, e)| e.value.as_ref().map(|v| (i as u32, e.generation, v)))
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_ids_miss() {
        let mut s = Slab::new();
        let (a, ga) = s.insert(1);
        let (b, gb) = s.insert(2);
        assert_eq!(s.len(), 2);
        assert_eq!(s.remove(a, ga), Some(1));
        assert_eq!(s.remove(a, ga), None);
        let (c, gc) = s.insert(3);
        assert_eq!(c, a);
        assert_ne!(gc, ga);
        assert_eq!(s.get(a, ga), None);
        assert_eq!(s.get(c, gc), Some(&3));
        *s.get_mut(b, gb).unwrap() = 20;
        assert_eq!(s.iter().map(|(_, _, v)| *v).collect::<Vec<_>>(), [3, 20]);
        assert_eq!(s.len(), 2);
    }
}
