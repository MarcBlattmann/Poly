/* futex_wake(uaddr, mask, nr, flags) (syscall 454) argument rules, with
 * no waiters: it returns 0 for a valid 32-bit futex (FUTEX2_SIZE_U32,
 * optionally FUTEX2_PRIVATE, FUTEX2_NUMA, FUTEX2_MPOL), whatever nr is.
 * EINVAL: a size other than U32, an unknown flag bit, a mask of 0 or with
 * bits above 31, an address not aligned to 4 bytes (8 with FUTEX2_NUMA).
 * EFAULT: an unmapped address of a shared futex; a private futex is not
 * looked up in memory, so NULL with FUTEX2_PRIVATE returns 0. flags is a
 * 32-bit value (bits 32..63 ignored). With FUTEX2_NUMA the word after the
 * futex holds a node: -1 is replaced by the caller's node (0 on a
 * one-node machine), a node that does not exist is EINVAL.
 */
#include "a_sys.h"

#define SYS_futex_wake 454
/* linux/futex.h */
#define FUTEX2_SIZE_U8 0x00
#define FUTEX2_SIZE_U16 0x01
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_SIZE_U64 0x03
#define FUTEX2_NUMA 0x04
#define FUTEX2_MPOL 0x08
#define FUTEX2_PRIVATE 128

static u32 word[16] __attribute__((aligned(64)));

static i64 wake(void *uaddr, u64 mask, i64 nr, u64 flags) {
    return syscall4(SYS_futex_wake, uaddr, mask, nr, flags);
}

int test_main(void) {
    u32 *w = word;
    expect("u32_no_waiters", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32), 0);
    expect("u32_private", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE), 0);
    expect("mask_one_bit", wake(w, 0x80000000, 1, FUTEX2_SIZE_U32), 0);
    expect("nr_0", wake(w, 0xffffffff, 0, FUTEX2_SIZE_U32), 0);
    expect("nr_negative", wake(w, 0xffffffff, -1, FUTEX2_SIZE_U32), 0);
    expect("nr_large", wake(w, 0xffffffff, 0x7fffffff, FUTEX2_SIZE_U32), 0);
    expect("flags_high_bits_ignored", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32 | (1UL << 32)), 0);
    expect("mpol_accepted", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_MPOL), 0);

    expect("size_u8_einval", wake(w, 0xff, 1, FUTEX2_SIZE_U8), -EINVAL);
    expect("size_u16_einval", wake(w, 0xffff, 1, FUTEX2_SIZE_U16), -EINVAL);
    expect("size_u64_einval", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U64), -EINVAL);
    expect("flag_0x10_einval", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32 | 0x10), -EINVAL);
    expect("flag_0x40_einval", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32 | 0x40), -EINVAL);
    expect("flag_0x100_einval", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32 | 0x100), -EINVAL);
    expect("mask_0_einval", wake(w, 0, 1, FUTEX2_SIZE_U32), -EINVAL);
    expect("mask_bit32_einval", wake(w, 1UL << 32, 1, FUTEX2_SIZE_U32), -EINVAL);
    expect("mask_all64_einval", wake(w, ~0UL, 1, FUTEX2_SIZE_U32), -EINVAL);
    expect("unaligned_1_einval", wake((char *)w + 1, 0xffffffff, 1, FUTEX2_SIZE_U32), -EINVAL);
    expect("unaligned_2_einval", wake((char *)w + 2, 0xffffffff, 1, FUTEX2_SIZE_U32), -EINVAL);
    expect("unaligned_private_einval", wake((char *)w + 2, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE),
           -EINVAL);

    expect("null_shared_efault", wake(0, 0xffffffff, 1, FUTEX2_SIZE_U32), -EFAULT);
    expect("low_page_shared_efault", wake((void *)0x1000, 0xffffffff, 1, FUTEX2_SIZE_U32), -EFAULT);
    expect("null_private_0", wake(0, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE), 0);
    /* Order: flags before the address. */
    expect("bad_size_null_einval", wake(0, 0xffffffff, 1, FUTEX2_SIZE_U8), -EINVAL);
    expect("bad_mask_null_einval", wake(0, 0, 1, FUTEX2_SIZE_U32), -EINVAL);

    /* FUTEX2_NUMA: the futex is followed by a 32-bit node number. */
    word[0] = 0;
    word[1] = 0xffffffff;
    expect("numa_ok", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_NUMA), 0);
    expect("numa_node_filled_in", word[1], 0);
    word[1] = 0;
    expect("numa_node_0", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_NUMA | FUTEX2_PRIVATE), 0);
    word[1] = 4096;
    expect("numa_bad_node_einval", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_NUMA), -EINVAL);
    word[1] = 0xfffffffe;
    expect("numa_node_minus2_einval", wake(w, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_NUMA), -EINVAL);
    word[3] = 0;
    expect("numa_align4_einval", wake(w + 1, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_NUMA), -EINVAL);
    return finish();
}
