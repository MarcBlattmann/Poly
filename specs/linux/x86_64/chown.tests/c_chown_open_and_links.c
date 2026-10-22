/* chown acts on the file, not the name: an open descriptor sees the
 * change (fstat), and so does every hard link to the file. An open
 * descriptor does not prevent the call. */
#include "a_sys.h"

#define D "/tmp/c_chown_open_and_links"

static i64 ch(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_hardlink", syscall2(SYS_link, D "/f", D "/h"), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("open", fd >= 0, fd);
    syscall2(SYS_chmod, D "/f", 04755);
    expect("chown_while_open", ch(D "/f", -1, -1), 0);
    syscall2(SYS_fstat, fd, &st);
    expect("fd_sees_change", st.st_mode & 07777, 0755);
    syscall2(SYS_chmod, D "/f", 04711);
    expect("chown_via_hardlink", ch(D "/h", -1, -1), 0);
    a_stat_path(D "/f", &st);
    expect("other_name_sees_change", st.st_mode & 07777, 0711);
    check("nlink", st.st_nlink == 2, st.st_nlink);
    a_close(fd);
    expect("cleanup_h", a_unlink(D "/h"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
