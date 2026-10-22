/* After fchmod, access checks use the new mode: a file with an execute
 * bit passes faccessat X_OK, one without fails with EACCES. Descriptors
 * opened before keep their access when the mode drops to 0. */
#include "a_sys.h"

#define F "/tmp/c_fchmod_access_effect"

static i64 x_ok(void) { return syscall3(SYS_faccessat, AT_FDCWD, F, X_OK); }

int test_main(void) {
    a_unlink(F);
    expect("setup_file", a_make_file(F, "abc", 0644), 0);
    i64 fd = a_open(F, O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("no_exec_at_start", x_ok(), -EACCES);
    expect("fchmod_0700", syscall2(SYS_fchmod, fd, 0700), 0);
    expect("exec_after_0700", x_ok(), 0);
    expect("fchmod_0600", syscall2(SYS_fchmod, fd, 0600), 0);
    expect("no_exec_after_0600", x_ok(), -EACCES);
    expect("fchmod_0755", syscall2(SYS_fchmod, fd, 0755), 0);
    expect("exec_after_0755", x_ok(), 0);
    expect("fchmod_04644", syscall2(SYS_fchmod, fd, 04644), 0);
    expect("suid_is_not_exec", x_ok(), -EACCES);
    expect("fchmod_0", syscall2(SYS_fchmod, fd, 0), 0);
    expect("no_exec_after_0", x_ok(), -EACCES);
    char c = 0;
    expect("old_fd_still_reads", a_read(fd, &c, 1), 1);
    expect("read_byte", c, 'a');
    expect("old_fd_still_writes", a_write(fd, "Z", 1), 1);
    expect("restore", syscall2(SYS_fchmod, fd, 0644), 0);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
