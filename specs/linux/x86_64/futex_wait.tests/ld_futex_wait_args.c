/* futex_wait(uaddr, val, mask, flags, timeout, clockid) (syscall 455)
 * argument rules and their order. Only 32-bit futexes (FUTEX2_SIZE_U32)
 * exist; flags may add FUTEX2_PRIVATE, FUTEX2_NUMA and FUTEX2_MPOL. The
 * order seen: flags (EINVAL), clockid when a timeout is given (only
 * CLOCK_MONOTONIC and CLOCK_REALTIME, else EINVAL), val above 32 bits
 * (EINVAL), the timeout itself (EFAULT unreadable, EINVAL negative or
 * tv_nsec out of range), mask (0 or above 32 bits: EINVAL) and alignment
 * (EINVAL), the address (EFAULT, also for a private futex), then the value
 * (EAGAIN) and last the deadline (ETIMEDOUT). Without a timeout the
 * clockid is not looked at.
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SYS_futex_wait 455
/* linux/futex.h */
#define FUTEX2_SIZE_U8 0x00
#define FUTEX2_SIZE_U16 0x01
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_SIZE_U64 0x03
#define FUTEX2_NUMA 0x04
#define FUTEX2_MPOL 0x08
#define FUTEX2_PRIVATE 128
#define U32 FUTEX2_SIZE_U32
#define ETIMEDOUT 110

static u32 word[16] __attribute__((aligned(64)));
static struct a_timespec zero = {0, 0}, neg = {-1, 0}, badns = {0, 1000000000}, negns = {0, -1};

static i64 fwait(void *uaddr, u64 val, u64 mask, u64 flags, void *ts, i64 clk) {
    return SC6(SYS_futex_wait, uaddr, val, mask, flags, ts, clk);
}

int test_main(void) {
    u32 *w = word;
    word[0] = 7;
    /* Results with valid arguments: a deadline already passed. */
    expect("deadline_passed_mono", fwait(w, 7, 0xffffffff, U32, &zero, CLOCK_MONOTONIC), -ETIMEDOUT);
    expect("deadline_passed_real", fwait(w, 7, 0xffffffff, U32, &zero, CLOCK_REALTIME), -ETIMEDOUT);
    expect("private_timedout", fwait(w, 7, 0xffffffff, U32 | FUTEX2_PRIVATE, &zero, CLOCK_MONOTONIC), -ETIMEDOUT);
    expect("mpol_timedout", fwait(w, 7, 0xffffffff, U32 | FUTEX2_MPOL, &zero, CLOCK_MONOTONIC), -ETIMEDOUT);
    expect("mask_one_bit", fwait(w, 7, 0x100, U32, &zero, CLOCK_MONOTONIC), -ETIMEDOUT);
    expect("flags_high_bits_ignored", fwait(w, 7, 0xffffffff, U32 | (1UL << 32), &zero, CLOCK_MONOTONIC), -ETIMEDOUT);
    expect("value_differs_eagain", fwait(w, 8, 0xffffffff, U32, 0, 0), -EAGAIN);
    expect("value_before_deadline", fwait(w, 8, 0xffffffff, U32, &zero, CLOCK_MONOTONIC), -EAGAIN);
    expect("no_timeout_clockid_ignored", fwait(w, 8, 0xffffffff, U32, 0, -1), -EAGAIN);

    /* Flags. */
    expect("size_u8", fwait(w, 7, 0xff, FUTEX2_SIZE_U8, &zero, 1), -EINVAL);
    expect("size_u16", fwait(w, 7, 0xffff, FUTEX2_SIZE_U16, &zero, 1), -EINVAL);
    expect("size_u64", fwait(w, 7, 0xffffffff, FUTEX2_SIZE_U64, &zero, 1), -EINVAL);
    expect("flag_0x10", fwait(w, 7, 0xffffffff, U32 | 0x10, &zero, 1), -EINVAL);
    expect("flag_0x100_clock_realtime_not_a_flag", fwait(w, 7, 0xffffffff, U32 | 0x100, &zero, 1), -EINVAL);
    /* Clocks. */
    expect("clock_boottime", fwait(w, 7, 0xffffffff, U32, &zero, CLOCK_BOOTTIME), -EINVAL);
    expect("clock_process_cputime", fwait(w, 7, 0xffffffff, U32, &zero, CLOCK_PROCESS_CPUTIME_ID), -EINVAL);
    expect("clock_tai", fwait(w, 7, 0xffffffff, U32, &zero, CLOCK_TAI), -EINVAL);
    expect("clock_minus1", fwait(w, 7, 0xffffffff, U32, &zero, -1), -EINVAL);
    expect("bad_clock_before_value", fwait(w, 8, 0xffffffff, U32, &zero, -1), -EINVAL);
    /* val, timeout, mask, alignment, address. */
    expect("val_above_32_bits", fwait(w, 7 | (1UL << 32), 0xffffffff, U32, &zero, 1), -EINVAL);
    expect("timeout_unreadable", fwait(w, 8, 0xffffffff, U32, (void *)1, 1), -EFAULT);
    expect("timeout_negative", fwait(w, 8, 0xffffffff, U32, &neg, 1), -EINVAL);
    expect("timeout_nsec_1e9", fwait(w, 8, 0xffffffff, U32, &badns, 1), -EINVAL);
    expect("timeout_nsec_negative", fwait(w, 8, 0xffffffff, U32, &negns, 1), -EINVAL);
    expect("mask_0", fwait(w, 7, 0, U32, &zero, 1), -EINVAL);
    expect("mask_bit32", fwait(w, 7, 1UL << 32, U32, &zero, 1), -EINVAL);
    expect("unaligned", fwait((char *)w + 1, 7, 0xffffffff, U32, &zero, 1), -EINVAL);
    expect("unaligned_2_private", fwait((char *)w + 2, 7, 0xffffffff, U32 | FUTEX2_PRIVATE, &zero, 1), -EINVAL);
    expect("null_shared", fwait(0, 7, 0xffffffff, U32, &zero, 1), -EFAULT);
    expect("null_private", fwait(0, 7, 0xffffffff, U32 | FUTEX2_PRIVATE, &zero, 1), -EFAULT);
    expect("low_page", fwait((void *)0x1000, 7, 0xffffffff, U32, &zero, 1), -EFAULT);
    /* Order between them. */
    expect("flags_before_timeout", fwait(w, 7, 0xffffffff, U32 | 0x10, (void *)1, 1), -EINVAL);
    expect("clock_before_timeout", fwait(w, 7, 0xffffffff, U32, (void *)1, -1), -EINVAL);
    expect("val_before_timeout", fwait(w, 1UL << 33, 0xffffffff, U32, (void *)1, 1), -EINVAL);
    expect("timeout_before_mask", fwait(w, 7, 0, U32, (void *)1, 1), -EFAULT);
    expect("timeout_before_alignment", fwait((char *)w + 1, 7, 0xffffffff, U32, (void *)1, 1), -EFAULT);
    expect("timeout_before_address", fwait(0, 7, 0xffffffff, U32, (void *)1, 1), -EFAULT);
    expect("mask_before_value", fwait(w, 8, 0, U32, &zero, 1), -EINVAL);
    expect("address_before_deadline", fwait(0, 7, 0xffffffff, U32 | FUTEX2_PRIVATE, &zero, 1), -EFAULT);

    /* FUTEX2_NUMA: the futex is followed by a node number (-1: any, filled in). */
    word[2] = 0;
    word[3] = 0xffffffff;
    expect("numa_timedout", fwait(w + 2, 0, 0xffffffff, U32 | FUTEX2_NUMA, &zero, 1), -ETIMEDOUT);
    expect("numa_node_filled_in", word[3], 0);
    word[3] = 4096;
    expect("numa_bad_node", fwait(w + 2, 0, 0xffffffff, U32 | FUTEX2_NUMA, &zero, 1), -EINVAL);
    word[3] = 0;
    expect("numa_needs_8_byte_alignment", fwait(w + 1, 0, 0xffffffff, U32 | FUTEX2_NUMA, &zero, 1), -EINVAL);
    return finish();
}
