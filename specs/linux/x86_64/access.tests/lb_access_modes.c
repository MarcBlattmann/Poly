/* access(2): permission bits against the caller's real uid, file and
 * directory, and the mode argument (F_OK, R_OK 4, W_OK 2, X_OK 1; any other
 * bit is EINVAL, checked before the path). Root passes R_OK and W_OK always,
 * X_OK on a directory always and on another file only when some execute bit
 * is set; a normal owner gets exactly the owner bits. */
#include "a_sys.h"

#define D "/tmp/lb_access_modes"

static i64 acc(const char *p, i64 mode) { return syscall2(SYS_access, p, mode); }
static i64 chmod_(const char *p, i64 mode) { return syscall2(SYS_chmod, p, mode); }

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);

    /* The mode argument: only the low three bits may be set. */
    expect("mode_8_einval", acc(D "/f", 8), -EINVAL);
    expect("mode_0x10_einval", acc(D "/f", 0x10), -EINVAL);
    expect("mode_minus1_einval", acc(D "/f", -1), -EINVAL);
    expect("mode_0x100_einval", acc(D "/f", 0x100), -EINVAL);
    /* checked before the path: a missing, empty or unreadable path is
     * still EINVAL */
    expect("mode_einval_before_enoent", acc(D "/nope", 8), -EINVAL);
    expect("mode_einval_before_empty", acc("", 8), -EINVAL);
    expect("mode_einval_before_efault", acc((const char *)0, 8), -EINVAL);
    /* the upper 32 bits of the register are ignored (the mode is an int) */
    expect("mode_high_bits_ignored", acc(D "/f", (i64)0x100000000L | R_OK), 0);

    /* 0644 */
    expect("f644_fok", acc(D "/f", F_OK), 0);
    expect("f644_r", acc(D "/f", R_OK), 0);
    expect("f644_w", acc(D "/f", W_OK), 0);
    expect("f644_rw", acc(D "/f", R_OK | W_OK), 0);
    expect("f644_x", acc(D "/f", X_OK), -EACCES);
    expect("f644_rwx", acc(D "/f", R_OK | W_OK | X_OK), -EACCES);

    /* 0400: read only */
    expect("chmod_400", chmod_(D "/f", 0400), 0);
    expect("f400_r", acc(D "/f", R_OK), 0);
    expect("f400_w", acc(D "/f", W_OK), root ? 0 : -EACCES);
    expect("f400_x", acc(D "/f", X_OK), -EACCES);

    /* 0200: write only */
    expect("chmod_200", chmod_(D "/f", 0200), 0);
    expect("f200_r", acc(D "/f", R_OK), root ? 0 : -EACCES);
    expect("f200_w", acc(D "/f", W_OK), 0);
    expect("f200_rw", acc(D "/f", R_OK | W_OK), root ? 0 : -EACCES);

    /* 0100: execute only */
    expect("chmod_100", chmod_(D "/f", 0100), 0);
    expect("f100_x", acc(D "/f", X_OK), 0);
    expect("f100_r", acc(D "/f", R_OK), root ? 0 : -EACCES);

    /* 0000: nothing, F_OK still answers */
    expect("chmod_000", chmod_(D "/f", 0), 0);
    expect("f000_fok", acc(D "/f", F_OK), 0);
    expect("f000_r", acc(D "/f", R_OK), root ? 0 : -EACCES);
    expect("f000_w", acc(D "/f", W_OK), root ? 0 : -EACCES);
    expect("f000_x", acc(D "/f", X_OK), -EACCES);

    /* 0001 and 0010: an execute bit of another class; the owner (a normal
     * user) gets only the owner bits, root passes X_OK on any x bit */
    expect("chmod_001", chmod_(D "/f", 01), 0);
    expect("f001_x", acc(D "/f", X_OK), root ? 0 : -EACCES);
    expect("chmod_010", chmod_(D "/f", 010), 0);
    expect("f010_x", acc(D "/f", X_OK), root ? 0 : -EACCES);
    /* 0077: everything for group and others, nothing for the owner */
    expect("chmod_077", chmod_(D "/f", 077), 0);
    expect("f077_r", acc(D "/f", R_OK), root ? 0 : -EACCES);
    expect("f077_x", acc(D "/f", X_OK), root ? 0 : -EACCES);

    /* set-id and sticky bits do not grant anything */
    expect("chmod_7000", chmod_(D "/f", 07000), 0);
    expect("f7000_x", acc(D "/f", X_OK), -EACCES);
    expect("f7000_fok", acc(D "/f", F_OK), 0);

    /* Directories: X_OK means search. */
    expect("dir_mkdir", a_mkdir(D "/d", 0700), 0);
    expect("d700_rwx", acc(D "/d", R_OK | W_OK | X_OK), 0);
    expect("chmod_d000", chmod_(D "/d", 0), 0);
    expect("d000_fok", acc(D "/d", F_OK), 0);
    expect("d000_x", acc(D "/d", X_OK), root ? 0 : -EACCES);
    expect("d000_rw", acc(D "/d", R_OK | W_OK), root ? 0 : -EACCES);
    expect("chmod_d500", chmod_(D "/d", 0500), 0);
    expect("d500_rx", acc(D "/d", R_OK | X_OK), 0);
    expect("d500_w", acc(D "/d", W_OK), root ? 0 : -EACCES);
    expect("d500_trailing_slash", acc(D "/d/", R_OK | X_OK), 0);

    /* Character device /dev/null (0666): never executable, even for root. */
    expect("devnull_rw", acc("/dev/null", R_OK | W_OK), 0);
    expect("devnull_x", acc("/dev/null", X_OK), -EACCES);

    expect("cleanup_rmdir_d", a_rmdir(D "/d"), 0);
    expect("cleanup_unlink", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
