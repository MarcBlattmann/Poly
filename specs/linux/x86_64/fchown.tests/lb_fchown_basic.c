/* fchown(2): change the owner and group of the file open on a descriptor.
 * Any access mode (and a directory) works, also after the last name is
 * gone; -1 leaves an id; the same rules as chown (a normal user keeps the
 * file and may only pick its own group; root sets anything); set-user-ID
 * is cleared on a regular file. Not-open and O_PATH descriptors are EBADF. */
#include "a_sys.h"

#define D "/tmp/lb_fchown_basic"

static i64 fchown_(i64 fd, i64 u, i64 g) { return syscall3(SYS_fchown, fd, u, g); }
static void fids(i64 fd, i64 *u, i64 *g) {
    struct a_stat st;
    *u = *g = -1;
    if (syscall2(SYS_fstat, fd, &st) == 0) *u = st.st_uid, *g = st.st_gid;
}

int test_main(void) {
    i64 uid = syscall0(SYS_getuid), gid = syscall0(SYS_getgid);
    int root = uid == 0;
    i64 u, g;
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    i64 r = a_open(D "/f", O_RDONLY, 0);
    i64 w = a_open(D "/f", O_WRONLY, 0);
    check("opened", r >= 0 && w >= 0, w);

    expect("rdonly_noop", fchown_(r, -1, -1), 0);
    expect("rdonly_own", fchown_(r, uid, gid), 0);
    expect("wronly_own", fchown_(w, uid, -1), 0);
    expect("give_away", fchown_(r, 6001, 6002), root ? 0 : -EPERM);
    fids(w, &u, &g);
    expect("uid_after", u, root ? 6001 : uid);
    expect("gid_after", g, root ? 6002 : gid);
    expect("path_agrees", a_stat_path(D "/f", &st) == 0 && st.st_uid == (u64)u, 1);
    expect("back", fchown_(w, uid, gid), 0);
    expect("upper_half_ignored", fchown_(w, (i64)0x300000000L | uid, -1), 0);

    /* set-user-ID cleared, set-group-ID without group x kept */
    expect("chmod_6745", syscall2(SYS_fchmod, r, 06745), 0);
    expect("fchown_clears", fchown_(r, -1, -1), 0);
    expect("mode_after", syscall2(SYS_fstat, r, &st) == 0 ? (i64)(st.st_mode & 07777) : -1, 02745);
    expect("chmod_6755", syscall2(SYS_fchmod, r, 06755), 0);
    expect("fchown_clears_both", fchown_(r, uid, gid), 0);
    expect("mode_after_both", syscall2(SYS_fstat, r, &st) == 0 ? (i64)(st.st_mode & 07777) : -1, 0755);

    /* directory descriptor */
    expect("mkdir_sub", a_mkdir(D "/sub", 0755), 0);
    i64 dfd = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    check("open_dir", dfd >= 0, dfd);
    expect("dir_chmod", syscall2(SYS_fchmod, dfd, 06755), 0);
    expect("dir_fchown", fchown_(dfd, uid, gid), 0);
    expect("dir_bits_kept", syscall2(SYS_fstat, dfd, &st) == 0 ? (i64)(st.st_mode & 07777) : -1, 06755);

    /* unlinked file */
    expect("unlink_f", a_unlink(D "/f"), 0);
    expect("unlinked_fchown", fchown_(w, uid, gid), 0);
    expect("unlinked_give_away", fchown_(w, 6003, -1), root ? 0 : -EPERM);

    /* bad descriptors */
    expect("minus1_ebadf", fchown_(-1, -1, -1), -EBADF);
    expect("atfdcwd_ebadf", fchown_(AT_FDCWD, -1, -1), -EBADF);
    expect("big_ebadf", fchown_(4000, -1, -1), -EBADF);
    expect("close_r", a_close(r), 0);
    expect("closed_ebadf", fchown_(r, -1, -1), -EBADF);
    i64 p = a_open(D "/sub", O_PATH, 0);
    check("open_opath", p >= 0, p);
    expect("opath_ebadf", fchown_(p, -1, -1), -EBADF);

    expect("close_p", a_close(p), 0);
    expect("close_dfd", a_close(dfd), 0);
    expect("close_w", a_close(w), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
