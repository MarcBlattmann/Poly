/* fchmod(fd, mode) sets the 12 permission bits of the open file to
 * exactly mode & 07777 and returns 0. The umask does not apply; the file
 * type is kept. Set-user-ID, set-group-ID and sticky bits are kept for
 * the owner when the file's group is the caller's own (a new file's
 * group is). */
#include "a_sys.h"

#define F "/tmp/c_fchmod_basic"

static i64 fmode(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    a_unlink(F);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_EXCL, 0600);
    check("setup_open", fd >= 0, fd);
    i64 old = syscall1(SYS_umask, 0777);
    expect("fchmod_0777", syscall2(SYS_fchmod, fd, 0777), 0);
    expect("umask_not_applied", fmode(fd), S_IFREG | 0777);
    syscall1(SYS_umask, old);
    expect("fchmod_0", syscall2(SYS_fchmod, fd, 0), 0);
    expect("mode_0", fmode(fd), S_IFREG);
    expect("fchmod_0644", syscall2(SYS_fchmod, fd, 0644), 0);
    expect("mode_0644", fmode(fd), S_IFREG | 0644);
    expect("fchmod_04755", syscall2(SYS_fchmod, fd, 04755), 0);
    expect("suid_kept", fmode(fd), S_IFREG | 04755);
    expect("fchmod_02750", syscall2(SYS_fchmod, fd, 02750), 0);
    expect("sgid_kept_own_group", fmode(fd), S_IFREG | 02750);
    expect("fchmod_01600", syscall2(SYS_fchmod, fd, 01600), 0);
    expect("sticky_on_file_kept", fmode(fd), S_IFREG | 01600);
    expect("fchmod_07777", syscall2(SYS_fchmod, fd, 07777), 0);
    expect("all_12_bits", fmode(fd), S_IFREG | 07777);
    /* Each permission bit on its own round-trips. */
    int ok = 1;
    for (i64 bit = 1; bit <= 04000; bit <<= 1) {
        ok &= syscall2(SYS_fchmod, fd, bit) == 0;
        ok &= fmode(fd) == (S_IFREG | bit);
    }
    check("single_bits_round_trip", ok, 0);
    struct a_stat st;
    expect("fchmod_0640", syscall2(SYS_fchmod, fd, 0640), 0);
    expect("stat_by_path", a_stat_path(F, &st), 0);
    expect("path_sees_mode", st.st_mode, S_IFREG | 0640);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
