/* The namespace files of another process: /proc/<pid>/ns/<type> of a
 * child that made UTS, network and IPC namespaces. readlink gives
 * "<type>:[<inode>]" with the inode of the opened file, different from
 * the caller's for the namespaces the child made and equal for the ones
 * it shares (mnt, user, pid, cgroup); lstat shows a symbolic link of mode
 * 0777 and size 0, stat the regular file of mode 0444; opening it for
 * writing (O_WRONLY, O_RDWR) is EPERM. A process with
 * another uid (65534) may not follow another user's links (readlink
 * EACCES) but may read its own. Another root process joins the child's
 * namespaces through the files (setns): its host name becomes the child's
 * and its network and IPC namespace inodes are the child's. A descriptor
 * kept open keeps the namespace usable after the child has ended and been
 * waited for, while /proc/<pid>/ns itself is gone (ENOENT).
 * poly-diff: needs=root (new namespaces and setns need CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (several processes, pids in /proc paths)
 */
#include "p_ns.h"

static i64 kid;
static char kpath[64];
static i64 kid_ino[3]; /* uts, net, ipc of the child */

static const char *kns(const char *name) { return p_proc_path(kpath, kid, name); }

static int nobody_reader(void) {
    char link[64];
    expect("drop", p_drop_to_nobody(), 0);
    expect("readlink_own_uts_ok", p_readlink("/proc/self/ns/uts", link, sizeof link) > 0, 1);
    expect("readlink_roots_uts", p_readlink(kns("ns/uts"), link, sizeof link), -EACCES);
    expect("readlink_roots_net", p_readlink(kns("ns/net"), link, sizeof link), -EACCES);
    expect("open_roots_uts", a_open(kns("ns/uts"), O_RDONLY, 0), -EACCES);
    return failed ? 1 : 0;
}

static int joiner(void) {
    struct a_utsname u;
    i64 uts = a_open(kns("ns/uts"), O_RDONLY, 0);
    i64 net = a_open(kns("ns/net"), O_RDONLY, 0);
    i64 ipc = a_open(kns("ns/ipc"), O_RDONLY, 0);
    check("opened", uts >= 0 && net >= 0 && ipc >= 0, uts);
    expect("setns_uts", syscall2(SYS_setns, uts, CLONE_NEWUTS), 0);
    expect("setns_net", syscall2(SYS_setns, net, CLONE_NEWNET), 0);
    expect("setns_ipc", syscall2(SYS_setns, ipc, 0), 0);
    syscall1(SYS_uname, &u);
    check("hostname_other", str_eq(u.nodename, "other"), 0);
    expect("uts_inode", p_ns_ino("/proc/self/ns/uts"), kid_ino[0]);
    expect("net_inode", p_ns_ino("/proc/self/ns/net"), kid_ino[1]);
    expect("ipc_inode", p_ns_ino("/proc/self/ns/ipc"), kid_ino[2]);
    return failed ? 1 : 0;
}

static int late_joiner(void) {
    struct a_utsname u;
    i64 fd = (i64)a_shared()[6];
    expect("setns_after_exit", syscall2(SYS_setns, fd, CLONE_NEWUTS), 0);
    syscall1(SYS_uname, &u);
    check("hostname_other_after_exit", str_eq(u.nodename, "other"), 0);
    return failed ? 1 : 0;
}

int test_main(void) {
    char link[64], own[64], path[64];
    struct a_stat st;
    a_block_sigchld();
    a_shared();
    i32 go[2], rdy[2];
    p_pipe(go);
    p_pipe(rdy);
    kid = p_clone(CLONE_NEWUTS | CLONE_NEWNET | CLONE_NEWIPC);
    if (kid == 0) {
        a_close(go[1]);
        a_close(rdy[0]);
        syscall2(SYS_sethostname, "other", 5);
        p_send_byte(rdy[1], 'r');
        char c;
        a_read(go[0], &c, 1);
        p_exit(0);
    }
    a_close(go[0]);
    a_close(rdy[1]);
    expect("child_ready", p_wait_byte(rdy[0]), 'r');
    kid_ino[0] = p_ns_ino(kns("ns/uts"));
    kid_ino[1] = p_ns_ino(kns("ns/net"));
    kid_ino[2] = p_ns_ino(kns("ns/ipc"));

    expect("readlink_uts", p_readlink(kns("ns/uts"), link, sizeof link) > 5, 1);
    check("uts_link_prefix", p_starts(link, "uts:["), 0);
    check("uts_link_suffix", link[str_len(link) - 1] == ']', 0);
    p_readlink("/proc/self/ns/uts", own, sizeof own);
    check("uts_differs_from_own", !str_eq(link, own), 0);
    p_readlink(kns("ns/net"), link, sizeof link);
    p_readlink("/proc/self/ns/net", own, sizeof own);
    check("net_link_prefix", p_starts(link, "net:["), 0);
    check("net_differs_from_own", !str_eq(link, own), 0);
    p_readlink(kns("ns/ipc"), link, sizeof link);
    check("ipc_link_prefix", p_starts(link, "ipc:["), 0);
    for (int i = 0; i < 4; i++) {
        const char *shared[] = {"mnt", "user", "pid", "cgroup"};
        char l[48], o[48];
        str_cat2(path, "ns/", shared[i]);
        p_readlink(kns(path), l, sizeof l);
        str_cat2(path, "/proc/self/ns/", shared[i]);
        p_readlink(path, o, sizeof o);
        char name[40];
        str_cat2(name, "shared_ns_equal_", shared[i]);
        check(name, str_eq(l, o), 0);
    }
    expect("link_inode_matches_stat", a_atou(link + 5), (u64)kid_ino[2]);

    expect("lstat_link", a_lstat_path(kns("ns/uts"), &st), 0);
    expect("lstat_is_symlink", st.st_mode & S_IFMT, S_IFLNK);
    expect("lstat_mode_0777", st.st_mode & 07777, 0777);
    expect("lstat_size_0", st.st_size, 0);
    expect("stat_file", a_stat_path(kns("ns/uts"), &st), 0);
    expect("stat_is_regular", st.st_mode & S_IFMT, S_IFREG);
    expect("stat_mode_0444", st.st_mode & 07777, 0444);
    expect("open_wronly", a_open(kns("ns/uts"), O_WRONLY, 0), -EPERM);
    expect("open_rdwr", a_open(kns("ns/uts"), O_RDWR, 0), -EPERM);

    p_run("nobody_exit_0", 0, nobody_reader);
    p_run("joiner_exit_0", 0, joiner);

    i64 held = a_open(kns("ns/uts"), O_RDONLY, 0);
    a_shared()[6] = (i32)held;
    a_write(go[1], "g", 1);
    a_close(go[1]);
    expect("child_exit_0", p_wait(kid), 0);
    expect("proc_entry_gone", a_open(kns("ns/uts"), O_RDONLY, 0), -ENOENT);
    p_run("late_joiner_exit_0", 0, late_joiner);
    return finish();
}
