/* unshare(CLONE_NEWUSER) puts the caller in a new user namespace that it
 * owns. Until a uid/gid mapping is written, its ids read as the overflow
 * ids (65534, /proc/sys/kernel/overflowuid and overflowgid) whatever they
 * were. Writing "0 <old euid> 1" to /proc/self/uid_map makes getuid and
 * geteuid 0; a second write is EPERM. gid_map can only be written after
 * "deny" was written to /proc/self/setgroups (EPERM before). The caller
 * has every capability in the namespace: it may then create a UTS
 * namespace (owned by the new user namespace) and set its host name,
 * but not change the name of the UTS namespace it came from (EPERM).
 * A user namespace cannot be made by a chrooted process (EPERM).
 * poly-diff: needs=root (whether a normal user may create user namespaces is system configuration)
 */
#include "a_sys.h"

#define SYS_chroot 161
#define SYS_sethostname 170
#define SYS_unshare 272
/* linux/sched.h */
#define CLONE_NEWUTS 0x04000000
#define CLONE_NEWUSER 0x10000000

static i64 write_file(const char *path, const char *s) {
    i64 fd = a_open(path, O_WRONLY, 0);
    if (fd < 0) return fd;
    i64 n = a_write(fd, s, str_len(s));
    a_close(fd);
    return n;
}

static int in_userns(void) {
    char map[48], num[24];
    i64 euid = syscall0(SYS_geteuid), egid = syscall0(SYS_getegid);
    expect("unshare_newuser", syscall1(SYS_unshare, CLONE_NEWUSER), 0);
    expect("uid_overflow", syscall0(SYS_getuid), 65534);
    expect("euid_overflow", syscall0(SYS_geteuid), 65534);
    expect("gid_overflow", syscall0(SYS_getgid), 65534);

    str_cpy(str_cpy(str_cpy(map, "0 "), a_utoa(num, (u64)euid)), " 1\n");
    expect("uid_map_written", write_file("/proc/self/uid_map", map), (i64)str_len(map));
    expect("uid_map_second_write_eperm", write_file("/proc/self/uid_map", map), -EPERM);
    expect("uid_now_0", syscall0(SYS_getuid), 0);
    expect("euid_now_0", syscall0(SYS_geteuid), 0);

    str_cpy(str_cpy(str_cpy(map, "0 "), a_utoa(num, (u64)egid)), " 1\n");
    expect("gid_map_before_deny_eperm", write_file("/proc/self/gid_map", map), -EPERM);
    expect("setgroups_deny", write_file("/proc/self/setgroups", "deny"), 4);
    expect("gid_map_written", write_file("/proc/self/gid_map", map), (i64)str_len(map));
    expect("gid_now_0", syscall0(SYS_getgid), 0);

    expect("old_uts_eperm", syscall2(SYS_sethostname, "x", 1), -EPERM);
    expect("unshare_newuts", syscall1(SYS_unshare, CLONE_NEWUTS), 0);
    expect("own_uts_sethostname", syscall2(SYS_sethostname, "userns", 6), 0);
    struct a_utsname u;
    syscall1(SYS_uname, &u);
    check("uname_userns", str_eq(u.nodename, "userns"), 0);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

static int chrooted(void) {
    expect("chroot", syscall1(SYS_chroot, "/tmp/ld_unshare_userns_root"), 0);
    expect("newuser_after_chroot_eperm", syscall1(SYS_unshare, CLONE_NEWUSER), -EPERM);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

static void run(const char *name, int (*fn)(void)) {
    i32 p = passed, f = failed;
    passed = failed = 0;
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, fn());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    passed = p + a_shared()[0];
    failed = f + a_shared()[1];
    expect(name, st, 0);
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    run("userns_child_exit_0", in_userns);
    a_rmdir("/tmp/ld_unshare_userns_root");
    a_mkdir("/tmp/ld_unshare_userns_root", 0755);
    run("chroot_child_exit_0", chrooted);
    a_rmdir("/tmp/ld_unshare_userns_root");
    return finish();
}
