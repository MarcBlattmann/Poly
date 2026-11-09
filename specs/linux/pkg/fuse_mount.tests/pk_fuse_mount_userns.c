/* Who may mount fuse. A user without privilege (uid 65534) that made a
 * user and a mount namespace and mapped itself to uid/gid 0 there may open
 * /dev/fuse and mount "fuse" in the new mount namespace, with or without
 * MS_NOSUID|MS_NODEV, and with allow_other; user_id= and group_id= must
 * then be ids mapped in that user namespace (0 is, 5 is not: EINVAL). The
 * same caller outside any user namespace is refused (EPERM). The test runs
 * the unprivileged part in a child.
 * (user_namespaces(7), mount(2))
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_prctl 157
#define CLONE_NEWUSER 0x10000000
#define PR_SET_DUMPABLE 4
#define P "/tmp/pk_fuse_userns"

static void write_file(const char *path, const char *text) {
    i64 fd = a_open(path, O_WRONLY, 0);
    if (fd >= 0) {
        a_write(fd, text, str_len(text));
        a_close(fd);
    }
}

static i64 mount_with(const char *rest, u64 flags) {
    i64 fd = a_open("/dev/fuse", O_RDWR, 0);
    if (fd < 0) return fd;
    char o[120];
    char *p = str_cpy(o, "fd=");
    a_utoa(p, (u64)fd);
    p += str_len(p);
    str_cpy(p, rest);
    i64 r = syscall5(SYS_mount, "pk", P, "fuse", flags, o);
    if (r == 0) syscall2(SYS_umount2, P, MNT_DETACH);
    a_close(fd);
    return r;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(P);
    expect("mkdir", a_mkdir(P, 0755), 0);
    syscall1(SYS_alarm, 25);
    volatile i32 *r = a_shared();
    for (int i = 0; i < 16; i++) r[i] = -9999;
    i64 pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        syscall3(SYS_setresgid, 65534, 65534, 65534);
        syscall3(SYS_setresuid, 65534, 65534, 65534);
        syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
        /* without a user namespace */
        r[1] = (i32)mount_with(",rootmode=40000,user_id=65534,group_id=65534", MS_NOSUID | MS_NODEV);
        r[2] = (i32)syscall1(SYS_unshare, CLONE_NEWUSER | CLONE_NEWNS);
        write_file("/proc/self/setgroups", "deny");
        write_file("/proc/self/uid_map", "0 65534 1");
        write_file("/proc/self/gid_map", "0 65534 1");
        r[3] = (i32)syscall5(SYS_mount, 0, "/", 0, MS_REC | MS_PRIVATE, 0);
        r[4] = (i32)mount_with(",rootmode=40000,user_id=0,group_id=0", MS_NOSUID | MS_NODEV);
        r[5] = (i32)mount_with(",rootmode=40000,user_id=0,group_id=0", 0);
        r[6] = (i32)mount_with(",rootmode=40000,user_id=0,group_id=0,allow_other", MS_NOSUID | MS_NODEV);
        r[7] = (i32)mount_with(",rootmode=40000,user_id=5,group_id=0", MS_NOSUID | MS_NODEV);
        r[8] = (i32)mount_with(",rootmode=40000,user_id=0,group_id=5", MS_NOSUID | MS_NODEV);
        syscall1(SYS_exit_group, 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child_exit", status, 0);
    expect("unshare_user_mount_ns", r[2], 0);
    expect("no_userns_refused", r[1], -EPERM);
    expect("userns_mount_ok", r[4], 0);
    expect("userns_mount_without_nosuid_ok", r[5], 0);
    expect("userns_allow_other_ok", r[6], 0);
    expect("userns_unmapped_user_id", r[7], -EINVAL);
    expect("userns_unmapped_group_id", r[8], -EINVAL);
    a_rmdir(P);
    return finish();
}
