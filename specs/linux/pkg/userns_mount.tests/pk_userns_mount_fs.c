/* What root of a user namespace may mount. A process with uid 65534 and no
 * capabilities makes unshare(CLONE_NEWUSER | CLONE_NEWNS), maps itself to
 * root ("0 65534 1", setgroups denied) and makes its mounts private. Then:
 * a tmpfs, a ramfs and a devpts instance can be mounted (0) on a directory
 * of the old tree; the top directory of the tmpfs belongs to the mounter,
 * so stat shows uid 0 and mode 040755 for "mode=0755", and a file made in
 * it is owned by uid 0; file systems that need the initial user namespace
 * are refused with EPERM before the source is even looked at (ext4,
 * squashfs, devtmpfs, cgroup2, with the source /dev/null); a character
 * device node cannot be made (mknod EPERM, also on the tmpfs it mounted)
 * while a fifo and a regular file can; sethostname in the inherited UTS
 * namespace is EPERM; chroot into the tmpfs works. The same mount of a
 * tmpfs is EPERM for the process before the unshare (it has no
 * capabilities then).
 * poly-diff: needs=root (the test starts as root and becomes uid 65534; creating user namespaces is a system setting)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

static int body(void) {
    struct a_stat st;
    expect("drop", p_drop_to_nobody(), 0);
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
    expect("tmpfs_before_unshare", p_mount("pk", "/tmp/pk_umfs", "tmpfs", 0, P_NULL), -EPERM);
    expect("unshare", syscall1(SYS_unshare, CLONE_NEWUSER | CLONE_NEWNS), 0);
    expect("maps", p_write_maps(0, 65534, 65534, 1), 0);
    expect("private", p_mount("", "/", P_NULL, MS_REC | MS_PRIVATE, P_NULL), 0);

    expect("tmpfs", p_mount("pk", "/tmp/pk_umfs", "tmpfs", 0, "mode=0755"), 0);
    expect("tmpfs_stat", a_stat_path("/tmp/pk_umfs", &st), 0);
    expect("tmpfs_uid_0", st.st_uid, 0);
    expect("tmpfs_gid_0", st.st_gid, 0);
    expect("tmpfs_mode", st.st_mode, 040755);
    expect("mkfile", a_make_file("/tmp/pk_umfs/f", "x", 1), 0);
    expect("file_stat", a_stat_path("/tmp/pk_umfs/f", &st), 0);
    expect("file_uid_0", st.st_uid, 0);
    expect("mkdir", a_mkdir("/tmp/pk_umfs/d", 0755), 0);
    expect("ramfs", p_mount("pk", "/tmp/pk_umfs/d", "ramfs", 0, P_NULL), 0);
    expect("mkdir2", a_mkdir("/tmp/pk_umfs/p", 0755), 0);
    expect("devpts", p_mount("devpts", "/tmp/pk_umfs/p", "devpts", 0, P_NULL), 0);
    expect("mkdir3", a_mkdir("/tmp/pk_umfs/e", 0755), 0);

    expect("ext4_eperm", p_mount("/dev/null", "/tmp/pk_umfs/e", "ext4", 0, P_NULL), -EPERM);
    expect("squashfs_eperm", p_mount("/dev/null", "/tmp/pk_umfs/e", "squashfs", 0, P_NULL), -EPERM);
    expect("devtmpfs_eperm", p_mount("devtmpfs", "/tmp/pk_umfs/e", "devtmpfs", 0, P_NULL), -EPERM);
    expect("cgroup2_eperm", p_mount("cgroup2", "/tmp/pk_umfs/e", "cgroup2", 0, P_NULL), -EPERM);

    expect("mknod_chr_eperm", syscall3(SYS_mknod, "/tmp/pk_umfs/n", 020644, 0x103), -EPERM);
    expect("mknod_fifo", syscall3(SYS_mknod, "/tmp/pk_umfs/fifo", 010644, 0), 0);
    expect("mknod_reg", syscall3(SYS_mknod, "/tmp/pk_umfs/reg", 0100644, 0), 0);
    expect("sethostname_eperm", syscall2(SYS_sethostname, "x", 1), -EPERM);
    expect("chroot", syscall1(SYS_chroot, "/tmp/pk_umfs"), 0);
    expect("chdir_root", p_chdir("/"), 0);
    expect("file_visible", a_stat_path("/f", &st), 0);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    a_mkdir("/tmp/pk_umfs", 0755);
    p_run("userns_mount_fs", CLONE_NEWNS, body);
    a_rmdir("/tmp/pk_umfs");
    return finish();
}
