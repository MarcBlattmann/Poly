/* Option 1 with a name that is not exactly a known type is EINVAL: an
 * unknown name, the empty string, a different case, a prefix of a known
 * name, a known name with a trailing space or slash, and the "nodev"
 * marker of /proc/filesystems. No file system module is loaded by the
 * lookup: the count stays the same.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sysfs 139

static i64 sysfs3(i64 opt, i64 a, i64 b) { return syscall3(SYS_sysfs, opt, a, b); }

static void bad(const char *tag, const char *name) {
    i64 r = sysfs3(1, (i64)name, 0);
    check(tag, r == -EINVAL, r);
}

int test_main(void) {
    i64 n = sysfs3(3, 0, 0);
    bad("unknown", "no_such_fs_type");
    bad("empty", "");
    bad("upper_case", "PROC");
    bad("mixed_case", "Proc");
    bad("prefix", "pro");
    bad("trailing_space", "proc ");
    bad("leading_space", " proc");
    bad("trailing_slash", "proc/");
    bad("tab_prefixed", "\tproc");
    bad("nodev_marker", "nodev");
    bad("dot", ".");
    bad("xfs_not_loaded_or_known", "xfs_definitely_not");
    i64 r = sysfs3(3, 0, 0);
    check("count_unchanged", r == n, r);
    r = sysfs3(1, (i64)"proc", 0);
    check("proc_still_ok", r >= 0, r);
    return finish();
}
