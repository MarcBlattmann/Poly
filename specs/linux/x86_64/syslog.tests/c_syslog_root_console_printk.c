/* syslog(2) as root and the first field of /proc/sys/kernel/printk (the
 * current console level): CONSOLE_OFF (6) lowers it to the minimum console
 * level (the third field), CONSOLE_ON (7) puts back the value OFF saved;
 * CONSOLE_LEVEL (8) sets it to len (1..8). The original level is restored
 * at the end. Skips the checks (as passes) if the level read is not 1..8.
 * poly-diff: needs=root (only a privileged caller may change the console level)
 */
#include "poly_ipc.h"

#define SYS_syslog 103

/* Fields of /proc/sys/kernel/printk; returns how many were parsed. */
static int read_printk(i64 f[4]) {
    char b[64];
    i64 fd = syscall2(SYS_open, "/proc/sys/kernel/printk", 0);
    if (fd < 0) return 0;
    i64 n = syscall3(SYS_read, fd, b, sizeof b - 1);
    syscall1(SYS_close, fd);
    if (n <= 0) return 0;
    int k = 0;
    i64 i = 0;
    while (k < 4 && i < n) {
        while (i < n && (b[i] == ' ' || b[i] == '\t')) i++;
        if (i >= n || b[i] < '0' || b[i] > '9') break;
        i64 v = 0;
        while (i < n && b[i] >= '0' && b[i] <= '9') v = v * 10 + (b[i++] - '0');
        f[k++] = v;
    }
    return k;
}

int test_main(void) {
    i64 f[4] = { 0 };
    int k = read_printk(f);
    check("printk_has_four_fields", k == 4, k);
    i64 orig = f[0], min = f[2];
    int usable = k == 4 && orig >= 1 && orig <= 8;

    i64 r = syscall3(SYS_syslog, 6, 0, 0);
    read_printk(f);
    check("console_off_sets_minimum", !usable || (r == 0 && f[0] == min), f[0]);
    r = syscall3(SYS_syslog, 7, 0, 0);
    read_printk(f);
    check("console_on_restores", !usable || (r == 0 && f[0] == orig), f[0]);

    i64 other = orig == 8 ? 7 : 8;
    r = usable ? syscall3(SYS_syslog, 8, 0, other) : 0;
    read_printk(f);
    check("console_level_sets_level", !usable || (r == 0 && f[0] == other), f[0]);
    r = usable ? syscall3(SYS_syslog, 8, 0, 10) : -EINVAL;
    read_printk(f);
    check("bad_level_leaves_level", !usable || (r == -EINVAL && f[0] == other), f[0]);

    /* OFF saves the level, CONSOLE_LEVEL forgets the saved one, ON then does nothing. */
    if (usable) {
        syscall3(SYS_syslog, 6, 0, 0);
        syscall3(SYS_syslog, 8, 0, orig);
        r = syscall3(SYS_syslog, 7, 0, 0);
    }
    read_printk(f);
    check("level_after_off_level_on", !usable || (r == 0 && f[0] == orig), f[0]);
    return finish();
}
