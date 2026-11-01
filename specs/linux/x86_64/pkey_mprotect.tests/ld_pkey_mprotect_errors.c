/* pkey_mprotect errors with key -1 (any CPU): EINVAL (22) for an addr
 * that is not page aligned (even with len 0), for prot bits other than
 * PROT_READ|WRITE|EXEC|SEM|GROWSDOWN|GROWSUP (0x10, bit 32: prot is a
 * full 64-bit value), for PROT_GROWSDOWN together with PROT_GROWSUP, and
 * for PROT_GROWSDOWN or PROT_GROWSUP on a mapping that does not grow;
 * ENOMEM (12) when the range is not entirely mapped (unmapped address,
 * kernel address, a hole) and when the rounded end wraps around (len
 * -8192, len ~0). Order: alignment, then len 0 (returns 0, also with bad
 * prot), then the wrap (ENOMEM before bad prot), then prot, then the
 * range.
 */
#include "a_sys.h"

#define SYS_pkey_mprotect 329

static i64 pm(void *a, u64 len, u64 prot, i64 key) { return syscall4(SYS_pkey_mprotect, a, len, prot, key); }

int test_main(void) {
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unaligned", pm(m + 1, A_PAGE, PROT_READ, -1), -EINVAL);
    expect("unaligned_len0", pm(m + 100, 0, PROT_READ, -1), -EINVAL);
    expect("bad_prot_0x10", pm(m, A_PAGE, 0x10, -1), -EINVAL);
    expect("bad_prot_bit32", pm(m, A_PAGE, (1UL << 32) | PROT_READ, -1), -EINVAL);
    expect("growsdown_and_up", pm(m, A_PAGE, PROT_READ | PROT_GROWSDOWN | PROT_GROWSUP, -1), -EINVAL);
    expect("growsdown_plain_mapping", pm(m, A_PAGE, PROT_READ | PROT_GROWSDOWN, -1), -EINVAL);
    expect("growsup_plain_mapping", pm(m, A_PAGE, PROT_READ | PROT_GROWSUP, -1), -EINVAL);
    expect("unmapped", pm((void *)0x10000, A_PAGE, PROT_READ, -1), -ENOMEM);
    expect("null", pm(0, A_PAGE, PROT_READ, -1), -ENOMEM);
    expect("kernel_address", pm((void *)0xffff800000000000UL, A_PAGE, PROT_READ, -1), -ENOMEM);
    expect("wrapping_len", pm(m, -8192L, PROT_READ, -1), -ENOMEM);
    expect("len_all_ones", pm(m, ~0UL, PROT_READ, -1), -ENOMEM);

    /* Order. */
    expect("unaligned_bad_prot", pm(m + 1, A_PAGE, 0x10, -1), -EINVAL);
    expect("len0_bad_prot", pm(m, 0, 0x10, -1), 0);
    expect("wrap_before_bad_prot", pm(m, -8192L, 0x10, -1), -ENOMEM);
    expect("bad_prot_before_unmapped", pm((void *)0x10000, A_PAGE, 0x10, -1), -EINVAL);

    char *h = (char *)syscall6(SYS_mmap, 0, 3 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, h + A_PAGE, A_PAGE);
    expect("starts_in_hole", pm(h + A_PAGE, 2 * A_PAGE, PROT_READ, -1), -ENOMEM);
    h[2 * A_PAGE] = 'z';
    check("page_after_hole_still_writable", h[2 * A_PAGE] == 'z', h[2 * A_PAGE]);
    expect("hole_page_only", pm(h + A_PAGE, A_PAGE, PROT_READ, -1), -ENOMEM);

    m[0] = 'k';
    check("m_unchanged", m[0] == 'k', m[0]);
    return finish();
}
