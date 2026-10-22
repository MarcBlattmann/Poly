/* fchmod works through a descriptor of any access mode (read-only,
 * write-only, read-write, append), a dup of one, and one opened with
 * O_CLOEXEC; the file's own permissions do not matter to its owner, so
 * a descriptor still works after the mode has gone to 0. */
#include "a_sys.h"

#define F "/tmp/c_fchmod_access_modes"

static i64 pmode(void) {
    struct a_stat st;
    i64 r = a_stat_path(F, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    a_unlink(F);
    expect("setup_file", a_make_file(F, "data", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("rdonly", syscall2(SYS_fchmod, fd, 0600), 0);
    expect("rdonly_mode", pmode(), S_IFREG | 0600);
    a_close(fd);
    fd = a_open(F, O_WRONLY, 0);
    expect("wronly", syscall2(SYS_fchmod, fd, 0604), 0);
    expect("wronly_mode", pmode(), S_IFREG | 0604);
    a_close(fd);
    fd = a_open(F, O_RDWR | O_APPEND, 0);
    expect("rdwr_append", syscall2(SYS_fchmod, fd, 0660), 0);
    expect("rdwr_append_mode", pmode(), S_IFREG | 0660);
    i64 d = syscall1(SYS_dup, fd);
    a_close(fd);
    expect("dup_of_closed_original", syscall2(SYS_fchmod, d, 0606), 0);
    expect("dup_mode", pmode(), S_IFREG | 0606);
    a_close(d);
    fd = a_open(F, O_RDONLY | O_CLOEXEC, 0);
    expect("cloexec", syscall2(SYS_fchmod, fd, 0), 0);
    expect("mode_0", pmode(), 0 | S_IFREG);
    expect("again_at_mode_0", syscall2(SYS_fchmod, fd, 0400), 0);
    expect("mode_0400", pmode(), S_IFREG | 0400);
    expect("readonly_fd_to_0", syscall2(SYS_fchmod, fd, 0), 0);
    expect("back_to_0644", syscall2(SYS_fchmod, fd, 0644), 0);
    expect("mode_0644", pmode(), S_IFREG | 0644);
    char buf[4];
    expect("contents_untouched", a_read(fd, buf, 4), 4);
    check("contents_equal", mem_eq(buf, "data", 4), 0);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
