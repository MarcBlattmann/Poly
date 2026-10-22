/* rmdir and the spelling of the last name: trailing slashes are allowed
 * on a directory ("D/a/", "D/a///"); repeated slashes count as one. A
 * last component "." is EINVAL ("D/a/.", ".", "D/a/./"), ".." is
 * ENOTEMPTY ("D/a/..", ".."), the root "/" (also "//", "/./" spelled
 * with only slashes) is EBUSY; "/." EINVAL, "/.." ENOTEMPTY. Nothing is
 * removed by any of these. "." and ".." in the middle resolve as usual. */
#include "a_sys.h"

#define D "/tmp/c_rmdir_dots"

static i64 rm(const char *p) { return syscall1(SYS_rmdir, p); }

int test_main(void) {
    struct a_stat st;
    expect("setup", a_mkdir(D, 0755), 0);

    expect("mk_a", a_mkdir(D "/a", 0755), 0);
    expect("trailing_slash", rm(D "/a/"), 0);
    expect("mk_a2", a_mkdir(D "/a", 0755), 0);
    expect("trailing_slashes", rm(D "/a///"), 0);
    expect("mk_a3", a_mkdir(D "/a", 0755), 0);
    expect("double_slashes_inside", rm("/tmp//c_rmdir_dots///a"), 0);
    expect("mk_a4", a_mkdir(D "/a", 0755), 0);
    expect("leading_double_slash", rm("//tmp/c_rmdir_dots/a"), 0);

    expect("mk_e", a_mkdir(D "/e", 0755), 0);
    expect("last_dot_einval", rm(D "/e/."), -EINVAL);
    expect("last_dot_slash_einval", rm(D "/e/./"), -EINVAL);
    expect("last_dot_slashes_einval", rm(D "/e/.//"), -EINVAL);
    expect("last_dotdot_enotempty", rm(D "/e/.."), -ENOTEMPTY);
    expect("last_dotdot_slash_enotempty", rm(D "/e/../"), -ENOTEMPTY);
    expect("e_kept", a_stat_path(D "/e", &st), 0);
    expect("missing_then_dot_enoent", rm(D "/no/."), -ENOENT);
    expect("missing_then_dotdot_enoent", rm(D "/no/.."), -ENOENT);

    /* Root and its spellings. */
    expect("root_ebusy", rm("/"), -EBUSY);
    expect("root_double_slash_ebusy", rm("//"), -EBUSY);
    expect("root_triple_slash_ebusy", rm("///"), -EBUSY);
    expect("root_dot_einval", rm("/."), -EINVAL);
    expect("root_dotdot_enotempty", rm("/.."), -ENOTEMPTY);
    expect("root_dotdot_dotdot_enotempty", rm("/../.."), -ENOTEMPTY);
    expect("root_tmp_dotdot_enotempty", rm("/tmp/.."), -ENOTEMPTY);

    /* Dots in the middle. */
    expect("mid_dot", rm(D "/./e"), 0);
    expect("mk_f", a_mkdir(D "/f", 0755), 0);
    expect("mk_g", a_mkdir(D "/g", 0755), 0);
    expect("mid_dotdot", rm(D "/g/../f"), 0);
    expect("f_gone", a_lstat_path(D "/f", &st), -ENOENT);
    expect("g_kept", a_lstat_path(D "/g", &st), 0);
    expect("mid_dotdot_self", rm(D "/g/../g"), 0);
    expect("mk_h", a_mkdir(D "/h", 0755), 0);
    expect("dotdot_above_root", rm("/../../tmp/c_rmdir_dots/h"), 0);

    /* Relative "." and ".." from the working directory. */
    expect("chdir_D", syscall1(SYS_chdir, D), 0);
    expect("dot_einval", rm("."), -EINVAL);
    expect("dot_slash_einval", rm("./"), -EINVAL);
    expect("dotdot_enotempty", rm(".."), -ENOTEMPTY);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);

    expect("cleanup_rmdir", rm(D), 0);
    return finish();
}
