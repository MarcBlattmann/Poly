/* lchown(2) on a regular file the caller owns: setting the caller's own
 * uid and gid, -1 for either id (leave it as it is), and only the low 32
 * bits of each argument counting (uid_t and gid_t are 32 bits). Ownership
 * needs no permission on the file itself (mode 0 works). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_lchown 94
#define D "/tmp/a_lchown_basic"

static i64 lchown(const char *p, i64 uid, i64 gid) { return syscall3(SYS_lchown, p, uid, gid); }

static int owned_by(const char *p, i64 uid, i64 gid) {
    struct a_stat st;
    return a_lstat_path(p, &st) == 0 && st.st_uid == (u32)uid && st.st_gid == (u32)gid;
}

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    check("created_with_caller_ids", owned_by(D "/f", uid, gid), 0);

    expect("self_both", lchown(D "/f", uid, gid), 0);
    check("self_both_ids", owned_by(D "/f", uid, gid), 0);
    expect("minus1_both", lchown(D "/f", -1, -1), 0);
    check("minus1_both_ids", owned_by(D "/f", uid, gid), 0);
    expect("self_uid_only", lchown(D "/f", uid, -1), 0);
    expect("self_gid_only", lchown(D "/f", -1, gid), 0);
    check("one_id_ids", owned_by(D "/f", uid, gid), 0);
    /* 0xffffffff is -1 as a 32-bit id. */
    expect("u32_minus1_both", lchown(D "/f", 0xffffffffL, 0xffffffffL), 0);
    check("u32_minus1_ids", owned_by(D "/f", uid, gid), 0);
    /* Bits above 32 are ignored. */
    expect("high_bits_ignored", lchown(D "/f", (1L << 32) | uid, (7L << 33) | gid), 0);
    check("high_bits_ids", owned_by(D "/f", uid, gid), 0);
    expect("high_bits_minus1", lchown(D "/f", (1L << 40) | 0xffffffffL, -1), 0);

    /* No read or write permission is needed on the file. */
    expect("setup_chmod_0", syscall2(SYS_chmod, D "/f", 0), 0);
    expect("mode_0_file", lchown(D "/f", uid, gid), 0);
    struct a_stat st;
    a_lstat_path(D "/f", &st);
    expect("mode_0_kept", st.st_mode & 07777, 0);
    expect("size_kept", st.st_size, 4);

    /* Directories work the same. */
    expect("setup_subdir", a_mkdir(D "/sub", 0700), 0);
    expect("dir_self", lchown(D "/sub", uid, gid), 0);
    expect("dir_minus1", lchown(D "/sub", -1, -1), 0);
    check("dir_ids", owned_by(D "/sub", uid, gid), 0);
    expect("dot", lchown(D "/.", -1, -1), 0);
    expect("dotdot", lchown(D "/sub/..", -1, -1), 0);
    expect("double_slash", lchown(D "//f", -1, -1), 0);

    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
