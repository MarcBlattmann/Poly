/* syslog(2) as root, argument checks of the reading actions 2 (READ),
 * 3 (READ_ALL) and 4 (READ_CLEAR), none of which then reads or clears
 * anything: a NULL buffer is EINVAL (even with length 0), a negative
 * length is EINVAL, length 0 with a buffer returns 0 at once (READ does
 * not block), a kernel-half buffer is EFAULT. len is an int: only its low
 * 32 bits count, so 0x100000000 is length 0.
 * poly-diff: needs=root (only a privileged caller gets past the permission check)
 */
#include "poly_ipc.h"

#define SYS_syslog 103

/* "<prefix>_<what>" in out. */
static const char *named(char *out, const char *prefix, const char *what) {
    size_t n = str_len(prefix), m = str_len(what);
    memcpy(out, prefix, n);
    out[n] = '_';
    memcpy(out + n + 1, what, m + 1);
    return out;
}

int test_main(void) {
    char buf[64];
    static const int types[] = { 2, 3, 4 };
    static const char *const names[] = { "read", "read_all", "read_clear" };
    char name[64];
    for (int i = 0; i < 3; i++) {
        int t = types[i];
        i64 r = syscall3(SYS_syslog, t, 0, 16);
        check(named(name, names[i], "null_buf_einval"), r == -EINVAL, r);
        r = syscall3(SYS_syslog, t, 0, 0);
        check(named(name, names[i], "null_buf_zero_len_einval"), r == -EINVAL, r);
        r = syscall3(SYS_syslog, t, buf, -1);
        check(named(name, names[i], "negative_len_einval"), r == -EINVAL, r);
        r = syscall3(SYS_syslog, t, buf, 0x80000000UL);
        check(named(name, names[i], "int_min_len_einval"), r == -EINVAL, r);
        r = syscall3(SYS_syslog, t, buf, 0);
        check(named(name, names[i], "zero_len_returns_0"), r == 0, r);
        r = syscall3(SYS_syslog, t, buf, 0x100000000UL);
        check(named(name, names[i], "len_high_bits_ignored"), r == 0, r);
        r = syscall3(SYS_syslog, t, 0xffff800000000000UL, 16);
        check(named(name, names[i], "kernel_buf_efault"), r == -EFAULT, r);
    }
    return finish();
}
