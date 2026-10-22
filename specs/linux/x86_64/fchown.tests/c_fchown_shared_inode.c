/* fchown acts on the file, not on the descriptor: the result (cleared
 * set-user-ID/set-group-ID bits, ids) is seen through other descriptors,
 * a hard link and the path. It also works on a file that has been
 * unlinked while still open. */
#include "a_sys.h"

#define D "/tmp/c_fchown_shared_inode"

static i64 pmode(const char *p) {
    struct a_stat st;
    i64 r = a_stat_path(p, &st);
    return r < 0 ? r : (i64)(st.st_mode & 07777);
}

static i64 fmode(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)(st.st_mode & 07777);
}

int test_main(void) {
    struct a_stat st;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    a_unlink(D "/f");
    a_unlink(D "/link");
    a_unlink(D "/gone");
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 a = a_open(D "/f", O_RDWR | O_CREAT | O_EXCL, 0644);
    check("open_a", a >= 0, a);
    i64 b = a_open(D "/f", O_RDONLY, 0);
    check("open_b", b >= 0, b);
    expect("link", syscall2(SYS_link, D "/f", D "/link"), 0);
    expect("chmod_06755", syscall2(SYS_fchmod, a, 06755), 0);
    expect("fchown_through_b", syscall3(SYS_fchown, b, uid, gid), 0);
    expect("mode_via_a", fmode(a), 00755);
    expect("mode_via_path", pmode(D "/f"), 00755);
    expect("mode_via_link", pmode(D "/link"), 00755);
    expect("chmod_04711", syscall2(SYS_fchmod, b, 04711), 0);
    expect("fchown_through_a", syscall3(SYS_fchown, a, -1, -1), 0);
    expect("mode_via_b", fmode(b), 00711);
    a_close(a);
    a_close(b);

    i64 g = a_open(D "/gone", O_RDWR | O_CREAT | O_EXCL, 0644);
    check("open_gone", g >= 0, g);
    expect("unlink_gone", a_unlink(D "/gone"), 0);
    expect("chmod_unlinked", syscall2(SYS_fchmod, g, 06755), 0);
    expect("fchown_unlinked", syscall3(SYS_fchown, g, uid, -1), 0);
    expect("unlinked_mode", fmode(g), 00755);
    expect("unlinked_stat", syscall2(SYS_fstat, g, &st), 0);
    expect("unlinked_nlink", (i64)st.st_nlink, 0);
    expect("unlinked_uid", (i64)st.st_uid, uid);
    a_close(g);
    expect("cleanup_link", a_unlink(D "/link"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
