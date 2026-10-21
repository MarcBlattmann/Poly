//! `qsort` and `bsearch` (C11 7.22.5) over raw elements of `size` bytes
//! with a caller's comparison (`qsort_s` passes a context too, which the
//! closure carries).
//!
//! `qsort` is an introsort: quicksort with a median-of-three pivot,
//! insertion sort for short runs, heapsort when the recursion gets too
//! deep, so it is O(n log n) at worst and never calls the comparison with
//! an element outside the array. It is not stable (C11 leaves the order of
//! equal elements unspecified).

/// Swap two distinct `size`-byte elements.
///
/// # Safety
/// `a` and `b` are valid for `size` bytes and do not overlap.
unsafe fn swap(a: *mut u8, b: *mut u8, size: usize) {
    // SAFETY: the caller's contract.
    unsafe { core::ptr::swap_nonoverlapping(a, b, size) }
}

/// Sort `n` elements of `size` bytes at `base` with `cmp` (negative, zero,
/// positive).
///
/// # Safety
/// `base` holds `n × size` writable bytes.
pub unsafe fn qsort(base: *mut u8, n: usize, size: usize, cmp: &mut dyn FnMut(*const u8, *const u8) -> i32) {
    if n < 2 || size == 0 {
        return;
    }
    let depth = 2 * (usize::BITS - n.leading_zeros()) as usize;
    // SAFETY: the caller's contract.
    unsafe { intro(base, n, size, cmp, depth) }
}

/// # Safety
/// As [`qsort`].
unsafe fn intro(
    base: *mut u8,
    mut n: usize,
    size: usize,
    cmp: &mut dyn FnMut(*const u8, *const u8) -> i32,
    mut depth: usize,
) {
    let mut base = base;
    loop {
        if n <= 16 {
            // SAFETY: the caller's contract.
            unsafe { insertion(base, n, size, cmp) };
            return;
        }
        if depth == 0 {
            // SAFETY: as above.
            unsafe { heapsort(base, n, size, cmp) };
            return;
        }
        depth -= 1;
        // SAFETY: every index below is < n.
        let at = |i: usize| unsafe { base.add(i * size) };
        // Median of three to the front.
        let (a, b, c) = (0, n / 2, n - 1);
        // SAFETY: distinct in-range elements.
        unsafe {
            if cmp(at(b), at(a)) < 0 {
                swap(at(a), at(b), size);
            }
            if cmp(at(c), at(b)) < 0 {
                swap(at(b), at(c), size);
                if cmp(at(b), at(a)) < 0 {
                    swap(at(a), at(b), size);
                }
            }
            swap(at(0), at(b), size);
        }
        // Hoare partition around the pivot at 0.
        let (mut i, mut j) = (0usize, n);
        loop {
            loop {
                i += 1;
                // SAFETY: i < n (the median-of-three put an element >= the
                // pivot at n - 1, stopping this scan).
                if i >= n || cmp(at(i), at(0)) >= 0 {
                    break;
                }
            }
            loop {
                j -= 1;
                // SAFETY: j >= 0, the pivot at 0 stops the scan.
                if j == 0 || cmp(at(j), at(0)) <= 0 {
                    break;
                }
            }
            if i >= j {
                break;
            }
            // SAFETY: distinct in-range elements.
            unsafe { swap(at(i), at(j), size) };
        }
        if j != 0 {
            // SAFETY: distinct in-range elements.
            unsafe { swap(at(0), at(j), size) };
        }
        // Recurse into the smaller side, loop on the larger.
        let (left_n, right_start) = (j, j + 1);
        let right_n = n - right_start;
        if left_n < right_n {
            // SAFETY: a sub-range of the array.
            unsafe { intro(base, left_n, size, cmp, depth) };
            base = at(right_start);
            n = right_n;
        } else {
            // SAFETY: as above.
            unsafe { intro(at(right_start), right_n, size, cmp, depth) };
            n = left_n;
        }
    }
}

/// # Safety
/// As [`qsort`].
unsafe fn insertion(base: *mut u8, n: usize, size: usize, cmp: &mut dyn FnMut(*const u8, *const u8) -> i32) {
    for i in 1..n {
        let mut j = i;
        // SAFETY: j and j - 1 are in range.
        while j > 0 && cmp(unsafe { base.add(j * size) }, unsafe { base.add((j - 1) * size) }) < 0 {
            // SAFETY: as above, distinct.
            unsafe { swap(base.add(j * size), base.add((j - 1) * size), size) };
            j -= 1;
        }
    }
}

/// # Safety
/// As [`qsort`].
unsafe fn heapsort(base: *mut u8, n: usize, size: usize, cmp: &mut dyn FnMut(*const u8, *const u8) -> i32) {
    // SAFETY: every index used is < n.
    let at = |i: usize| unsafe { base.add(i * size) };
    let sift = |cmp: &mut dyn FnMut(*const u8, *const u8) -> i32, mut root: usize, end: usize| loop {
        let mut child = 2 * root + 1;
        if child >= end {
            break;
        }
        if child + 1 < end && cmp(at(child), at(child + 1)) < 0 {
            child += 1;
        }
        if cmp(at(root), at(child)) >= 0 {
            break;
        }
        // SAFETY: distinct in-range elements.
        unsafe { swap(at(root), at(child), size) };
        root = child;
    };
    for start in (0..n / 2).rev() {
        sift(cmp, start, n);
    }
    for end in (1..n).rev() {
        // SAFETY: distinct in-range elements.
        unsafe { swap(at(0), at(end), size) };
        sift(cmp, 0, end);
    }
}

/// `bsearch`: an element comparing equal to the key, if any (`cmp(key,
/// element)`).
///
/// # Safety
/// `base` holds `n × size` readable bytes, sorted for `cmp`.
pub unsafe fn bsearch(
    base: *const u8,
    n: usize,
    size: usize,
    cmp: &mut dyn FnMut(*const u8) -> i32,
) -> Option<*const u8> {
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        // SAFETY: mid < n.
        let p = unsafe { base.add(mid * size) };
        match cmp(p) {
            0 => return Some(p),
            r if r < 0 => hi = mid,
            _ => lo = mid + 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn sort_i32(v: &mut [i32]) {
        let mut calls = 0usize;
        let n = v.len();
        let mut cmp = |a: *const u8, b: *const u8| {
            calls += 1;
            // SAFETY: elements of the slice.
            let (a, b) = unsafe { (*(a as *const i32), *(b as *const i32)) };
            a.cmp(&b) as i32
        };
        // SAFETY: the slice's bytes.
        unsafe { qsort(v.as_mut_ptr() as *mut u8, n, 4, &mut cmp) };
    }

    #[test]
    fn sorts_many_shapes() {
        let mut seed = 12345u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            seed >> 8
        };
        for n in [0usize, 1, 2, 3, 15, 16, 17, 100, 1000, 5000] {
            let shapes: [Vec<i32>; 5] = [
                (0..n).map(|_| (rnd() % 1000) as i32).collect(),
                (0..n as i32).collect(),
                (0..n as i32).rev().collect(),
                alloc::vec![7; n],
                (0..n).map(|i| (i % 3) as i32).collect(),
            ];
            for mut v in shapes {
                let mut want = v.clone();
                want.sort();
                sort_i32(&mut v);
                assert_eq!(v, want, "n = {n}");
            }
        }
    }

    #[test]
    fn odd_element_sizes_and_heapsort() {
        // 3-byte elements, sorted by their first byte.
        let mut v: Vec<u8> = (0..60u8).rev().flat_map(|i| [i, i ^ 0x55, 0xEE]).collect();
        let mut cmp = |a: *const u8, b: *const u8| {
            // SAFETY: elements of the vector.
            unsafe { i32::from(*a) - i32::from(*b) }
        };
        // SAFETY: the vector's bytes.
        unsafe { qsort(v.as_mut_ptr(), 60, 3, &mut cmp) };
        for (i, e) in v.chunks(3).enumerate() {
            assert_eq!(e, [i as u8, i as u8 ^ 0x55, 0xEE]);
        }
        // The heapsort fallback on its own.
        let mut w: Vec<i32> = (0..100).map(|i| (i * 37) % 101).collect();
        let mut cmp = |a: *const u8, b: *const u8| {
            // SAFETY: elements of the vector.
            unsafe { (*(a as *const i32)).cmp(&*(b as *const i32)) as i32 }
        };
        // SAFETY: the vector's bytes.
        unsafe { heapsort(w.as_mut_ptr() as *mut u8, 100, 4, &mut cmp) };
        assert!(w.windows(2).all(|p| p[0] <= p[1]));
    }

    #[test]
    fn binary_search() {
        let v: Vec<i32> = (0..50).map(|i| i * 2).collect();
        let find = |key: i32| {
            let mut cmp = |p: *const u8| {
                // SAFETY: an element of the vector.
                key.cmp(unsafe { &*(p as *const i32) }) as i32
            };
            // SAFETY: the vector's bytes, sorted.
            unsafe { bsearch(v.as_ptr() as *const u8, v.len(), 4, &mut cmp) }
                .map(|p| (p as usize - v.as_ptr() as usize) / 4)
        };
        assert_eq!(find(0), Some(0));
        assert_eq!(find(98), Some(49));
        assert_eq!(find(40), Some(20));
        assert_eq!(find(41), None);
        assert_eq!(find(-1), None);
        // SAFETY: no elements.
        assert_eq!(unsafe { bsearch(v.as_ptr() as *const u8, 0, 4, &mut |_| 0) }, None);
    }
}
