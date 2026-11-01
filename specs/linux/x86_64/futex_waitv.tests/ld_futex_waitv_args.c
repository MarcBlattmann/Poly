/* futex_waitv(waiters, nr_futexes, flags, timeout, clockid) (syscall 449)
 * argument rules and order. struct futex_waitv is 24 bytes: u64 val,
 * u64 uaddr, u32 flags, u32 __reserved. Order seen: flags != 0, nr 0 or
 * above 128, waiters NULL (all EINVAL); clockid when a timeout is given
 * (CLOCK_MONOTONIC / CLOCK_REALTIME only, EINVAL); the timeout (EFAULT,
 * EINVAL); the array (EFAULT); every entry's flags (only FUTEX2_SIZE_U32
 * plus FUTEX2_PRIVATE/FUTEX2_NUMA/FUTEX2_MPOL), __reserved (must be 0),
 * val (no bits above 31) and alignment, all entries before any address is
 * used (EINVAL); then every entry's address (EFAULT), then every value
 * (EAGAIN, the first entry that differs), then the deadline (ETIMEDOUT).
 * nr_futexes and flags are 32 bits wide; the same address may appear
 * twice.
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SC5(n, a, b, c, d, e) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), 0)
#define SYS_futex_waitv 449
/* linux/futex.h */
#define FUTEX2_SIZE_U8 0x00
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_SIZE_U64 0x03
#define FUTEX2_NUMA 0x04
#define FUTEX2_PRIVATE 128
#define FUTEX_WAITV_MAX 128
#define ETIMEDOUT 110

struct futex_waitv {
    u64 val;
    u64 uaddr;
    u32 flags;
    u32 reserved;
};

static u32 word[64] __attribute__((aligned(64)));
static struct futex_waitv v[FUTEX_WAITV_MAX + 2];
static struct a_timespec zero = {0, 0}, neg = {-1, 0}, badns = {0, 1000000000};

static i64 waitv(void *w, u64 nr, u64 flags, void *ts, i64 clk) { return SC5(SYS_futex_waitv, w, nr, flags, ts, clk); }

static void set(int i, u32 *addr, u64 val, u32 flags) {
    v[i].val = val;
    v[i].uaddr = (u64)addr;
    v[i].flags = flags;
    v[i].reserved = 0;
}

int test_main(void) {
    for (int i = 0; i < 8; i++) word[i] = (u32)i;
    set(0, word + 1, 1, FUTEX2_SIZE_U32);
    set(1, word + 2, 2, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE);
    expect("one_timedout", waitv(v, 1, 0, &zero, CLOCK_MONOTONIC), -ETIMEDOUT);
    expect("two_timedout_realtime", waitv(v, 2, 0, &zero, CLOCK_REALTIME), -ETIMEDOUT);
    expect("nr_high_bits_ignored", waitv(v, (1UL << 32) | 2, 0, &zero, CLOCK_MONOTONIC), -ETIMEDOUT);
    expect("flags_high_bits_ignored", waitv(v, 2, 1UL << 32, &zero, CLOCK_MONOTONIC), -ETIMEDOUT);
    expect("no_timeout_clockid_ignored", (v[1].val = 9, waitv(v, 2, 0, 0, -1)), -EAGAIN);
    v[1].val = 2;

    expect("flags_1", waitv(v, 1, 1, &zero, 1), -EINVAL);
    expect("nr_0", waitv(v, 0, 0, &zero, 1), -EINVAL);
    expect("nr_129", waitv(v, FUTEX_WAITV_MAX + 1, 0, &zero, 1), -EINVAL);
    expect("waiters_null", waitv(0, 1, 0, &zero, 1), -EINVAL);
    expect("clock_boottime", waitv(v, 1, 0, &zero, CLOCK_BOOTTIME), -EINVAL);
    expect("clock_minus1", waitv(v, 1, 0, &zero, -1), -EINVAL);
    expect("timeout_unreadable", waitv(v, 1, 0, (void *)1, 1), -EFAULT);
    expect("timeout_negative", waitv(v, 1, 0, &neg, 1), -EINVAL);
    expect("timeout_nsec_1e9", waitv(v, 1, 0, &badns, 1), -EINVAL);
    expect("waiters_unreadable", waitv((void *)0x1000, 1, 0, &zero, 1), -EFAULT);
    expect("waiters_unreadable_no_timeout", waitv((void *)0x1000, 1, 0, 0, 1), -EFAULT);
    expect("nr_before_null", waitv(0, 0, 0, &zero, 1), -EINVAL);
    expect("null_before_timeout", waitv(0, 1, 0, (void *)1, 1), -EINVAL);
    expect("clock_before_timeout", waitv(v, 1, 0, (void *)1, -1), -EINVAL);

    /* Entry fields. */
    v[0].flags = FUTEX2_SIZE_U8;
    expect("entry_size_u8", waitv(v, 1, 0, &zero, 1), -EINVAL);
    v[0].flags = FUTEX2_SIZE_U64;
    expect("entry_size_u64", waitv(v, 1, 0, &zero, 1), -EINVAL);
    v[0].flags = FUTEX2_SIZE_U32 | 0x10;
    expect("entry_flag_0x10", waitv(v, 1, 0, &zero, 1), -EINVAL);
    v[0].flags = FUTEX2_SIZE_U32;
    v[0].reserved = 1;
    expect("entry_reserved_nonzero", waitv(v, 1, 0, &zero, 1), -EINVAL);
    v[0].reserved = 0;
    v[0].val = 1 | (1UL << 32);
    expect("entry_val_above_32_bits", waitv(v, 1, 0, &zero, 1), -EINVAL);
    v[0].val = 1;
    v[0].uaddr = (u64)word + 1;
    expect("entry_unaligned", waitv(v, 1, 0, &zero, 1), -EINVAL);
    v[0].uaddr = 0;
    expect("entry_null_address", waitv(v, 1, 0, &zero, 1), -EFAULT);
    v[0].uaddr = 0x1000;
    expect("entry_low_page", waitv(v, 1, 0, &zero, 1), -EFAULT);

    /* Order between entries. */
    set(0, 0, 0, FUTEX2_SIZE_U32);
    set(1, word + 2, 2, FUTEX2_SIZE_U32 | 0x10);
    expect("bad_flags_later_beats_fault_earlier", waitv(v, 2, 0, &zero, 1), -EINVAL);
    set(0, word + 1, 5, FUTEX2_SIZE_U32);
    set(1, 0, 2, FUTEX2_SIZE_U32);
    expect("fault_later_beats_value_earlier", waitv(v, 2, 0, &zero, 1), -EFAULT);
    set(1, word + 2, 2, FUTEX2_SIZE_U32);
    expect("value_beats_deadline", waitv(v, 2, 0, &zero, 1), -EAGAIN);

    /* The maximum, with one address many times. */
    for (int i = 0; i < FUTEX_WAITV_MAX; i++) set(i, word + 3, 3, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE);
    expect("max_128_same_address", waitv(v, FUTEX_WAITV_MAX, 0, &zero, 1), -ETIMEDOUT);
    v[100].val = 4;
    expect("max_128_one_differs", waitv(v, FUTEX_WAITV_MAX, 0, &zero, 1), -EAGAIN);
    v[100].val = 3;
    v[127].flags = FUTEX2_SIZE_U32 | 0x40;
    expect("max_128_last_bad_flags", waitv(v, FUTEX_WAITV_MAX, 0, &zero, 1), -EINVAL);

    /* FUTEX2_NUMA entries: a node number follows the futex. */
    word[8] = 0;
    word[9] = 0xffffffff;
    set(0, word + 8, 0, FUTEX2_SIZE_U32 | FUTEX2_NUMA);
    expect("numa_entry_timedout", waitv(v, 1, 0, &zero, 1), -ETIMEDOUT);
    expect("numa_node_filled_in", word[9], 0);
    word[9] = 4096;
    expect("numa_entry_bad_node", waitv(v, 1, 0, &zero, 1), -EINVAL);
    return finish();
}
