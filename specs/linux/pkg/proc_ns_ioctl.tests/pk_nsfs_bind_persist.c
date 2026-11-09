/* A namespace can be kept alive by bind-mounting its /proc/<pid>/ns/<type>
 * file onto a regular file (what "ip netns add" does). In a private
 * mount namespace: a helper creates a UTS and a network namespace, names
 * the host "persist" and bind-mounts /proc/self/ns/uts and
 * /proc/self/ns/net onto two files; the helper exits. The bind mount
 * works (MS_BIND on a namespace file onto a regular file), on a directory
 * it is ENOTDIR. The mounted file is a regular file of mode 0444 whose
 * inode number is the namespace's; opening it after its creator is gone
 * and calling setns on the descriptor joins the namespace (nstype 0 or
 * the type): the host name reads "persist" and /proc/self/ns/net has
 * the recorded inode; the caller's own namespaces are unchanged. After
 * umount2 the file is the plain empty file again and setns on it is
 * EINVAL.
 * poly-diff: needs=root (mount, new namespaces and setns need CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

#define BASE "/tmp/pk_nsbind"
#define UTSF BASE "/uts"
#define NETF BASE "/net"

static volatile u64 *slots(void) { return (volatile u64 *)(a_shared() + 8); }

static int helper(void) {
    expect("helper_unshare", syscall1(SYS_unshare, CLONE_NEWUTS | CLONE_NEWNET), 0);
    expect("helper_sethostname", syscall2(SYS_sethostname, "persist", 7), 0);
    slots()[0] = (u64)p_ns_ino("/proc/self/ns/uts");
    slots()[1] = (u64)p_ns_ino("/proc/self/ns/net");
    expect("bind_uts", p_mount("/proc/self/ns/uts", UTSF, 0, MS_BIND, 0), 0);
    expect("bind_net", p_mount("/proc/self/ns/net", NETF, 0, MS_BIND, 0), 0);
    expect("bind_onto_directory", p_mount("/proc/self/ns/uts", BASE "/dir", 0, MS_BIND, 0), -ENOTDIR);
    return failed ? 1 : 0;
}

static int joiner(void) {
    struct a_utsname u;
    i64 fd = a_open(UTSF, O_RDONLY, 0);
    check("open_uts_file", fd >= 0, fd);
    expect("setns_uts_typed", syscall2(SYS_setns, fd, CLONE_NEWUTS), 0);
    syscall1(SYS_uname, &u);
    check("hostname_persist", str_eq(u.nodename, "persist"), 0);
    a_close(fd);
    i64 net = a_open(NETF, O_RDONLY, 0);
    expect("setns_net_any_type", syscall2(SYS_setns, net, 0), 0);
    expect("net_inode_is_recorded", (u64)p_ns_ino("/proc/self/ns/net"), slots()[1]);
    expect("uts_inode_is_recorded", (u64)p_ns_ino("/proc/self/ns/uts"), slots()[0]);
    return failed ? 1 : 0;
}

static int in_private_mounts(void) {
    struct a_stat st;
    struct a_utsname u;
    expect("make_private", p_mount(0, "/", 0, MS_REC | MS_PRIVATE, 0), 0);
    a_rmdir(BASE "/dir");
    a_unlink(UTSF);
    a_unlink(NETF);
    a_rmdir(BASE);
    expect("mkdir_base", a_mkdir(BASE, 0755), 0);
    expect("mkdir_dir", a_mkdir(BASE "/dir", 0755), 0);
    expect("make_uts_file", a_make_file(UTSF, "", 0644), 0);
    expect("make_net_file", a_make_file(NETF, "", 0644), 0);
    u64 own_uts = (u64)p_ns_ino("/proc/self/ns/uts");

    p_run("helper_exit_0", 0, helper);
    syscall1(SYS_uname, &u);
    check("own_hostname_unchanged", !str_eq(u.nodename, "persist"), 0);

    expect("stat_uts_file", a_stat_path(UTSF, &st), 0);
    expect("uts_file_is_regular", st.st_mode & S_IFMT, S_IFREG);
    expect("uts_file_mode", st.st_mode & 07777, 0444);
    expect("uts_file_inode", (u64)st.st_ino, slots()[0]);
    check("uts_inode_not_ours", (u64)st.st_ino != own_uts, 0);

    p_run("joiner_exit_0", 0, joiner);
    check("still_in_own_uts", (u64)p_ns_ino("/proc/self/ns/uts") == own_uts, 0);

    expect("umount_uts", syscall2(SYS_umount2, UTSF, 0), 0);
    expect("umount_net", syscall2(SYS_umount2, NETF, 0), 0);
    i64 fd = a_open(UTSF, O_RDONLY, 0);
    expect("setns_plain_file", syscall2(SYS_setns, fd, 0), -EINVAL);
    a_close(fd);
    expect("unlink_uts", a_unlink(UTSF), 0);
    expect("unlink_net", a_unlink(NETF), 0);
    expect("rmdir_dir", a_rmdir(BASE "/dir"), 0);
    expect("rmdir_base", a_rmdir(BASE), 0);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    p_run("private_mounts_exit_0", CLONE_NEWNS, in_private_mounts);
    return finish();
}
