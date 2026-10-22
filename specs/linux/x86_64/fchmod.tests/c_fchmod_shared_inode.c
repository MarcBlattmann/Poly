/* fchmod changes the file, not the descriptor: every name (hard links)
 * and every other open of the same file sees the new mode at once, and
 * a later open does too. */
#include "a_sys.h"

#define D "/tmp/c_fchmod_shared_inode"

static i64 fmode(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

static i64 pmode(const char *p) {
    struct a_stat st;
    i64 r = a_stat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    a_unlink(D "/a");
    a_unlink(D "/b");
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/a", "", 0644), 0);
    expect("setup_hard_link", syscall5(SYS_linkat, AT_FDCWD, D "/a", AT_FDCWD, D "/b", 0), 0);
    i64 fa = a_open(D "/a", O_RDONLY, 0);
    i64 fb = a_open(D "/b", O_RDONLY, 0);
    check("setup_opens", fa >= 0 && fb >= 0, fa < 0 ? fa : fb);
    expect("fchmod_via_a", syscall2(SYS_fchmod, fa, 0751), 0);
    expect("name_a", pmode(D "/a"), S_IFREG | 0751);
    expect("name_b", pmode(D "/b"), S_IFREG | 0751);
    expect("other_open", fmode(fb), S_IFREG | 0751);
    expect("fchmod_via_b", syscall2(SYS_fchmod, fb, 0402), 0);
    expect("first_open_sees_b", fmode(fa), S_IFREG | 0402);
    i64 fc = a_open(D "/b", O_RDONLY, 0);
    expect("later_open", fmode(fc), S_IFREG | 0402);
    a_close(fc);
    expect("unlink_a", a_unlink(D "/a"), 0);
    expect("fchmod_after_name_gone", syscall2(SYS_fchmod, fa, 0620), 0);
    expect("name_b_after", pmode(D "/b"), S_IFREG | 0620);
    a_close(fa);
    a_close(fb);
    expect("cleanup_b", a_unlink(D "/b"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
