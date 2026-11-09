/* fsopen(fstype, flags) starts the creation of a filesystem and returns a
 * "filesystem context" descriptor: the lowest free descriptor, with
 * FD_CLOEXEC set only when FSOPEN_CLOEXEC (1) was given. Its link in
 * /proc/self/fd reads "anon_inode:[fscontext]". Nothing is mounted yet.
 * A fresh context has no messages: read gives ENODATA; it cannot seek
 * (ESPIPE). Several contexts can exist at once; each is independent, and
 * closing one without creating anything has no effect. Filesystem types
 * that exist are accepted without any parameters: tmpfs, proc, devpts,
 * overlay, squashfs, fuse.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fsopen_basic"

static void body(const char *base) {
    (void)base;
    i64 a = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    check("fsopen_cloexec_ok", a >= 3, a);
    i64 fd_flags = syscall2(SYS_fcntl, a, F_GETFD);
    expect("cloexec_set", fd_flags, FD_CLOEXEC);
    i64 b = pm_fsopen("tmpfs", 0);
    check("fsopen_plain_ok", b >= 3 && b != a, b);
    expect("cloexec_not_set", syscall2(SYS_fcntl, b, F_GETFD), 0);
    expect("lowest_free_fd", b, a + 1);

    char link[64], target[64];
    memset(target, 0, sizeof target);
    str_cat2(link, "/proc/self/fd/", "");
    char *e = link + str_len(link);
    a_utoa(e, (u64)a);
    i64 n = syscall3(SYS_readlink, link, target, sizeof target - 1);
    check("link_is_fscontext", n > 0 && str_eq(target, "anon_inode:[fscontext]"), n);

    char buf[64];
    expect("read_empty_enodata", a_read(a, buf, sizeof buf), -61);
    expect("seek_espipe", syscall3(SYS_lseek, a, 0, SEEK_SET), -ESPIPE);
    expect("contexts_independent_params", pm_fsconfig(a, FSCONFIG_SET_STRING, "size", "1m", 0), 0);
    expect("other_context_untouched", pm_fsconfig(b, FSCONFIG_SET_STRING, "size", "2m", 0), 0);

    expect("close_context", a_close(a), 0);
    expect("closed_fd_ebadf", pm_fsconfig(a, FSCONFIG_SET_STRING, "size", "1m", 0), -EBADF);
    expect("other_context_still_works", pm_fsconfig(b, FSCONFIG_CMD_CREATE, 0, 0, 0), 0);
    expect("close_other", a_close(b), 0);

    const char *types[] = {"tmpfs", "proc", "devpts", "overlay", "squashfs", "fuse"};
    for (int i = 0; i < 6; i++) {
        i64 fd = pm_fsopen(types[i], FSOPEN_CLOEXEC);
        check(types[i], fd >= 3, fd);
        if (fd >= 0) a_close(fd);
    }
}

int test_main(void) { return pm_main(BASE, body); }
