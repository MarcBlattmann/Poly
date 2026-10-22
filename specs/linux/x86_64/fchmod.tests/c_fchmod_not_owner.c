/* fchmod by a caller that does not own the file is EPERM, even when the
 * descriptor is open for writing, the file is world-writable, or the
 * mode would not change; the mode stays. A bad or O_PATH descriptor is
 * EBADF first. The checks run in a child; a root caller first gives up
 * root (to uid and gid 65534, no supplementary groups), so the files
 * used (/dev/null and /, owned by root) are never the caller's. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119

static i64 pmode(const char *p) {
    struct a_stat st;
    i64 r = a_stat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

static int body(void) {
    if (syscall0(SYS_geteuid) == 0) {
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, 65534, 65534, 65534);
        syscall3(SYS_setresuid, 65534, 65534, 65534);
    }
    /* Never touch /dev/null or / as root. */
    check("not_root", syscall0(SYS_geteuid) != 0, 0);
    if (syscall0(SYS_geteuid) == 0) return finish();
    i64 null_mode = pmode("/dev/null");
    i64 root_mode = pmode("/");
    i64 fd = a_open("/dev/null", O_RDONLY, 0);
    check("open_null_rdonly", fd >= 0, fd);
    expect("null_rdonly_eperm", syscall2(SYS_fchmod, fd, 0666), -EPERM);
    expect("null_same_mode_eperm", syscall2(SYS_fchmod, fd, null_mode & 07777), -EPERM);
    a_close(fd);
    fd = a_open("/dev/null", O_WRONLY, 0);
    check("open_null_wronly", fd >= 0, fd);
    expect("null_writable_still_eperm", syscall2(SYS_fchmod, fd, 0600), -EPERM);
    a_close(fd);
    fd = a_open("/dev/null", O_PATH, 0);
    expect("null_o_path_ebadf_first", syscall2(SYS_fchmod, fd, 0600), -EBADF);
    a_close(fd);
    fd = a_open("/", O_RDONLY | O_DIRECTORY, 0);
    check("open_root_dir", fd >= 0, fd);
    expect("root_dir_eperm", syscall2(SYS_fchmod, fd, 0777), -EPERM);
    a_close(fd);
    expect("null_mode_unchanged", pmode("/dev/null"), null_mode);
    expect("root_mode_unchanged", pmode("/"), root_mode);
    return finish();
}

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, body());
    i32 status = -1;
    if (pid < 0 || syscall4(SYS_wait4, pid, &status, 0, 0) != pid) return 1;
    return A_WIFEXITED(status) ? A_WEXITSTATUS(status) : 1;
}
