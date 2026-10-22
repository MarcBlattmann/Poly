/* fchown on a directory descriptor returns 0 and leaves every mode bit,
 * set-user-ID, set-group-ID and sticky included; the directory's ids
 * stay. */
#include "a_sys.h"

#define D "/tmp/c_fchown_dir_bits"

static i64 fmode(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    struct a_stat st;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("setup_open", fd >= 0, fd);
    expect("chmod_07777", syscall2(SYS_fchmod, fd, 07777), 0);
    expect("minus_one", syscall3(SYS_fchown, fd, -1, -1), 0);
    expect("mode_07777_kept", fmode(fd), S_IFDIR | 07777);
    expect("own_ids", syscall3(SYS_fchown, fd, uid, gid), 0);
    expect("mode_07777_still", fmode(fd), S_IFDIR | 07777);
    expect("chmod_02755", syscall2(SYS_fchmod, fd, 02755), 0);
    expect("own_uid", syscall3(SYS_fchown, fd, uid, -1), 0);
    expect("mode_02755_kept", fmode(fd), S_IFDIR | 02755);
    expect("chmod_06711", syscall2(SYS_fchmod, fd, 06711), 0);
    expect("own_gid", syscall3(SYS_fchown, fd, -1, gid), 0);
    expect("mode_06711_kept", fmode(fd), S_IFDIR | 06711);
    expect("stat", syscall2(SYS_fstat, fd, &st), 0);
    expect("dir_uid", (i64)st.st_uid, uid);
    expect("dir_gid", (i64)st.st_gid, gid);
    expect("chmod_back", syscall2(SYS_fchmod, fd, 0755), 0);
    a_close(fd);
    expect("cleanup", a_rmdir(D), 0);
    return finish();
}
