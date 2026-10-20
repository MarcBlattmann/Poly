/* faccessat: well-known paths, "." and "..", trailing slashes, and the
 * standard devices. */
#include "a_sys.h"

static i64 acc(const char *p, i64 mode) { return syscall4(SYS_faccessat, AT_FDCWD, p, mode, 0); }

int test_main(void) {
    expect("root_dir", acc("/", F_OK), 0);
    expect("root_dir_rwx", acc("/", R_OK | W_OK | X_OK), 0);
    expect("tmp", acc("/tmp", W_OK), 0);
    expect("tmp_trailing_slash", acc("/tmp/", F_OK), 0);
    expect("dot", acc(".", F_OK), 0);
    expect("dotdot_of_root", acc("/..", F_OK), 0);
    expect("dev_null_rw", acc("/dev/null", R_OK | W_OK), 0);
    expect("dev_null_no_exec", acc("/dev/null", X_OK), -EACCES);
    expect("dev_null_slash_enotdir", acc("/dev/null/", F_OK), -ENOTDIR);
    expect("missing_top", acc("/no_such_dir_here", F_OK), -ENOENT);
    expect("missing_deep", acc("/no_such_dir_here/x/y", F_OK), -ENOENT);
    expect("double_slashes", acc("//tmp//", F_OK), 0);
    return finish();
}
