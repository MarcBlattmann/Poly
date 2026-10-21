//! A table of values addressed by index and generation, so a stale id
//! never reaches a value that reused its slot.

use alloc::vec::Vec;

#[derive(Debug, Clone)]
struct Entry<T> {
    generation: u32,
    value: Option<T>,
}

#[derive(Debug, Clone)]
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
            self.slots.push(Entry { generation: 0, value: Some(value) });
            ((self.slots.len() - 1) as u32, 0)
        }
    }

    pub(crate) fn get(&self, index: u32, generation: u32) -> Option<&T> {
        self.slots.get(index as usize).filter(|e| e.generation == generation).and_then(|e| e.value.as_ref())
    }

    pub(crate) fn get_mut(&mut self, index: u32, generation: u32) -> Option<&mut T> {
        self.slots.get_mut(index as usize).filter(|e| e.generation == generation).and_then(|e| e.value.as_mut())
    }

    pub(crate) fn remove(&mut self, index: u32, generation: u32) -> Option<T> {
        let e = self.slots.get_mut(index as usize).filter(|e| e.generation == generation)?;
        let value = e.value.take()?;
        e.generation = e.generation.wrapping_add(1);
        self.free.push(index);
        self.len -= 1;
        Some(value)
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Every live value with its (index, generation).
    pub(crate) fn iter(&self) -> impl Iterator<Item = ((u32, u32), &T)> {
        self.slots.iter().enumerate().filter_map(|(i, e)| e.value.as_ref().map(|v| ((i as u32, e.generation), v)))
    }
}
