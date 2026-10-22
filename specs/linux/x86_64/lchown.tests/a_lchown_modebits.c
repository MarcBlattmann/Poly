/* Every successful lchown(2) of a regular file, even one with -1 for both
 * ids, clears the set-user-ID bit, and clears the set-group-ID bit when
 * group execute is set too (set-group-ID without group execute stays).
 * The sticky bit and the permission bits stay. Directories keep all their
 * mode bits, set-user-ID, set-group-ID and sticky included. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_lchown 94
#define D "/tmp/a_lchown_modebits"
#define F D "/f"

static i64 lchown(const char *p, i64 uid, i64 gid) { return syscall3(SYS_lchown, p, uid, gid); }

static i64 mode_of(const char *p) {
    struct a_stat st;
    if (a_lstat_path(p, &st) != 0) return -1;
    return st.st_mode & 07777;
}

/* chmod `p` to `before`, lchown it with (uid, gid), return the mode after. */
static i64 after(const char *p, i64 before, i64 uid, i64 gid) {
    if (syscall2(SYS_chmod, p, before) != 0) return -2;
    if (mode_of(p) != before) return -3;
    i64 r = lchown(p, uid, gid);
    if (r != 0) return r;
    return mode_of(p);
}

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);

    expect("suid_minus1", after(F, 04755, -1, -1), 0755);
    expect("suid_self_uid", after(F, 04755, uid, -1), 0755);
    expect("suid_self_gid", after(F, 04755, -1, gid), 0755);
    expect("suid_no_exec", after(F, 04644, -1, -1), 0644);
    expect("suid_sgid_gx", after(F, 06755, -1, -1), 0755);
    expect("suid_sgid_no_gx", after(F, 06745, -1, -1), 02745);
    expect("sgid_gx", after(F, 02775, uid, gid), 0775);
    expect("sgid_gx_only", after(F, 02010, -1, -1), 010);
    expect("sgid_no_gx", after(F, 02644, -1, -1), 02644);
    expect("sgid_no_gx_ox", after(F, 02701, -1, -1), 02701);
    expect("sticky_kept", after(F, 01755, -1, -1), 01755);
    expect("all_bits", after(F, 07777, -1, -1), 01777);
    expect("all_bits_no_gx", after(F, 07767, uid, gid), 03767);
    expect("plain_kept", after(F, 0751, -1, -1), 0751);
    expect("mode_0", after(F, 0, -1, -1), 0);

    expect("setup_dir", a_mkdir(D "/dir", 0755), 0);
    expect("dir_all_bits", after(D "/dir", 07777, -1, -1), 07777);
    expect("dir_suid_sgid", after(D "/dir", 06755, uid, gid), 06755);
    expect("dir_sgid", after(D "/dir", 02755, -1, -1), 02755);
    expect("dir_restore", syscall2(SYS_chmod, D "/dir", 0755), 0);

    /* A link has mode 0777 and keeps it; its target is untouched. */
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("setup_target_suid", syscall2(SYS_chmod, F, 06755), 0);
    expect("link_minus1", lchown(D "/l", -1, -1), 0);
    expect("link_mode", mode_of(D "/l"), 0777);
    expect("link_target_bits_kept", mode_of(F), 06755);

    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_dir", a_rmdir(D "/dir"), 0);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
