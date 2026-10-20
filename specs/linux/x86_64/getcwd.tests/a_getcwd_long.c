/* getcwd: a deep working directory (over 1000 bytes of path) is reported
 * whole; buffers just large enough and one byte short. */
#include "a_sys.h"

#define D "/tmp/a_getcwd_long"
#define NAME "dir_with_a_fairly_long_name_0123456789"
#define LEVELS 30

static char want[2048], got[2048];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("chdir_base", syscall1(SYS_chdir, D), 0);
    char *p = str_cpy(want, D);
    int ok = 1;
    for (int i = 0; i < LEVELS; i++) {
        ok &= a_mkdir(NAME, 0700) == 0 && syscall1(SYS_chdir, NAME) == 0;
        p = str_cpy(str_cpy(p, "/"), NAME);
    }
    check("make_deep_tree", ok, 0);
    i64 len = str_len(want) + 1;
    check("path_over_1000", len > 1000, len);
    expect("deep_length", syscall2(SYS_getcwd, got, sizeof got), len);
    check("deep_text", str_eq(got, want), 0);
    expect("deep_exact", syscall2(SYS_getcwd, got, len), len);
    expect("deep_short_erange", syscall2(SYS_getcwd, got, len - 1), -ERANGE);
    for (int i = 0; i < LEVELS; i++) {
        ok &= syscall1(SYS_chdir, "..") == 0 && a_rmdir(NAME) == 0;
    }
    check("remove_deep_tree", ok, 0);
    syscall2(SYS_getcwd, got, sizeof got);
    check("back_at_base", str_eq(got, D), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
