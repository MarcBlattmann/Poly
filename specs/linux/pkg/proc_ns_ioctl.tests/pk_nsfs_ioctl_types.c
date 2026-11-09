/* ioctls on a namespace file (/proc/self/ns/<type>, linux/nsfs.h):
 * NS_GET_NSTYPE (0xb703) returns the CLONE_NEW* constant of the
 * namespace's type (user 0x10000000, net 0x40000000, mnt 0x20000, uts
 * 0x04000000, ipc 0x08000000, pid and pid_for_children 0x20000000,
 * cgroup 0x02000000, time and time_for_children 0x80). NS_GET_USERNS
 * (0xb701) returns a new descriptor for the user namespace that owns the
 * namespace (for the net namespace of the initial user's process: the
 * user namespace the process is in; same inode), close-on-exec set. On
 * the initial user namespace and the initial pid namespace NS_GET_USERNS
 * resp. NS_GET_PARENT fail with EPERM (they have no owner / no parent);
 * NS_GET_PARENT on a net namespace is EINVAL. NS_GET_OWNER_UID (0xb704)
 * stores the owner's uid in an unsigned int for a user namespace and is
 * EINVAL for any other type. NS_GET_MNTNS_ID (0x8008b705) stores a
 * non-zero 64-bit id for a mount namespace, the same on every call and
 * different after unshare(CLONE_NEWNS); other types: EINVAL. An unknown
 * request number (0xb7ff) is ENOTTY. The descriptors work with O_RDONLY.
 * poly-diff: needs=root (new namespaces)
 * poly-diff: syscalls=off (several processes; descriptor numbers and ids vary)
 */
#include "p_ns.h"

#define NS_GET_MNTNS_ID 0x8008b705

static i64 ioctl2(i64 fd, u64 req, void *arg) { return syscall3(SYS_ioctl, fd, req, arg); }

static i64 open_ns(const char *name) {
    char path[48];
    str_cat2(path, "/proc/self/ns/", name);
    return a_open(path, O_RDONLY, 0);
}

static void nstype(const char *name, u64 want) {
    i64 fd = open_ns(name);
    char label[48];
    str_cat2(label, "nstype_", name);
    expect(label, ioctl2(fd, NS_GET_NSTYPE, 0), (i64)want);
    a_close(fd);
}

static i64 ino_of_fd(i64 fd) {
    struct a_stat st;
    if (syscall2(SYS_fstat, fd, &st) < 0) return -1;
    return (i64)st.st_ino;
}

int test_main(void) {
    nstype("user", CLONE_NEWUSER);
    nstype("net", CLONE_NEWNET);
    nstype("mnt", CLONE_NEWNS);
    nstype("uts", CLONE_NEWUTS);
    nstype("ipc", CLONE_NEWIPC);
    nstype("pid", CLONE_NEWPID);
    nstype("pid_for_children", CLONE_NEWPID);
    nstype("cgroup", CLONE_NEWCGROUP);
    nstype("time", CLONE_NEWTIME);
    nstype("time_for_children", CLONE_NEWTIME);

    i64 user = open_ns("user"), net = open_ns("net"), mnt = open_ns("mnt"), pid = open_ns("pid");
    i64 owner = ioctl2(net, NS_GET_USERNS, 0);
    check("net_userns_fd", owner >= 0, owner);
    expect("net_userns_same_inode_as_own", ino_of_fd(owner), ino_of_fd(user));
    expect("userns_fd_cloexec", syscall2(SYS_fcntl, owner, F_GETFD) & FD_CLOEXEC, FD_CLOEXEC);
    a_close(owner);
    expect("initial_user_has_no_owner", ioctl2(user, NS_GET_USERNS, 0), -EPERM);
    expect("initial_user_has_no_parent", ioctl2(user, NS_GET_PARENT, 0), -EPERM);
    expect("initial_pid_has_no_parent", ioctl2(pid, NS_GET_PARENT, 0), -EPERM);
    expect("net_has_no_parent", ioctl2(net, NS_GET_PARENT, 0), -EINVAL);
    expect("mnt_has_no_parent", ioctl2(mnt, NS_GET_PARENT, 0), -EINVAL);

    unsigned int uid = 12345;
    expect("owner_uid_of_initial_user", ioctl2(user, NS_GET_OWNER_UID, &uid), 0);
    expect("owner_uid_value", uid, 0);
    expect("owner_uid_of_net_einval", ioctl2(net, NS_GET_OWNER_UID, &uid), -EINVAL);

    u64 id1 = 0, id2 = 0, id3 = 0;
    expect("mntns_id", ioctl2(mnt, NS_GET_MNTNS_ID, &id1), 0);
    check("mntns_id_nonzero", id1 != 0, 0);
    expect("mntns_id_again", ioctl2(mnt, NS_GET_MNTNS_ID, &id2), 0);
    check("mntns_id_stable", id1 == id2, 0);
    expect("mntns_id_of_net_einval", ioctl2(net, NS_GET_MNTNS_ID, &id3), -EINVAL);
    expect("unknown_request", ioctl2(net, 0xb7ff, 0), -ENOTTY);

    /* A new mount namespace has another id. */
    a_block_sigchld();
    a_shared();
    i64 child = p_clone(CLONE_NEWNS);
    if (child == 0) {
        i64 m = open_ns("mnt");
        u64 id = 0;
        i64 r = ioctl2(m, NS_GET_MNTNS_ID, &id);
        a_shared()[2] = (i32)((r == 0 && id != 0 && id != id1) ? 1 : 0);
        p_exit(0);
    }
    p_wait(child);
    expect("new_mntns_other_id", a_shared()[2], 1);
    return finish();
}
