/* POSIX message queues and the per-namespace System V limits follow the
 * IPC namespace. In a child made with clone(CLONE_NEWIPC | CLONE_NEWNS)
 * (mounts made private) a queue made with mq_open("pk_mq...", O_CREAT |
 * O_RDWR) shows up as a file of that name in a mqueue file system mounted
 * in the namespace (mount type "mqueue", on a directory; the file is a
 * regular file of mode 0600). A process in a further IPC namespace
 * (unshare(CLONE_NEWIPC)) gets ENOENT from mq_open of that name; a mqueue
 * mounted there lists nothing (the directory is empty), while the mount
 * made earlier keeps showing the old namespace's queues, because a mqueue
 * mount belongs to the IPC namespace of its mounter. A queue of the same
 * name can be created in the inner namespace and is a different queue
 * (removing it with mq_unlink leaves the outer one). The limits in
 * /proc/sys/kernel/msgmni and /proc/sys/fs/mqueue/msg_max are per
 * namespace too: a value written in the inner namespace is read back
 * there and the outer one keeps its own.
 * poly-diff: needs=root (a new IPC namespace and a mount need CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

#define SYS_mq_open 240
#define SYS_mq_unlink 241

static char name[40];
static char buf[4096];

static int dir_entries(const char *path) {
    int num, all;
    if (p_count_dir(path, &num, &all) < 0) return -1;
    return all;
}

static int inner(void) {
    expect("inner_open_enoent", syscall4(SYS_mq_open, name, O_RDWR, 0, 0), -ENOENT);
    a_mkdir("/tmp/pk_ipcmq/b", 0755);
    expect("inner_mount", p_mount("mqueue", "/tmp/pk_ipcmq/b", "mqueue", 0, P_NULL), 0);
    expect("inner_listing_empty", dir_entries("/tmp/pk_ipcmq/b"), 2);
    check("old_mount_still_old", p_dir_has("/tmp/pk_ipcmq/a", name), 0);
    i64 fd = syscall4(SYS_mq_open, name, O_CREAT | O_RDWR, 0600, 0);
    check("inner_create", fd >= 0, fd);
    check("inner_listing_has_it", p_dir_has("/tmp/pk_ipcmq/b", name), 0);
    check("old_mount_unchanged_count", dir_entries("/tmp/pk_ipcmq/a") == 3, 0);
    expect("inner_unlink", syscall1(SYS_mq_unlink, name), 0);
    expect("inner_listing_empty_again", dir_entries("/tmp/pk_ipcmq/b"), 2);
    expect("inner_umount", syscall2(SYS_umount2, "/tmp/pk_ipcmq/b", 0), 0);
    a_rmdir("/tmp/pk_ipcmq/b");

    expect("inner_write_msgmni", p_write_file("/proc/sys/kernel/msgmni", "77"), 2);
    p_read_file("/proc/sys/kernel/msgmni", buf, sizeof buf);
    expect("inner_msgmni_read", (i64)p_num(buf), 77);
    expect("inner_write_msg_max", p_write_file("/proc/sys/fs/mqueue/msg_max", "11"), 2);
    p_read_file("/proc/sys/fs/mqueue/msg_max", buf, sizeof buf);
    expect("inner_msg_max_read", (i64)p_num(buf), 11);
    return failed ? 1 : 0;
}

static i64 outer_msgmni, outer_msg_max;

static int body(void) {
    expect("private", p_mount("", "/", P_NULL, MS_REC | MS_PRIVATE, P_NULL), 0);
    a_mkdir("/tmp/pk_ipcmq", 0755);
    a_mkdir("/tmp/pk_ipcmq/a", 0755);
    expect("mount", p_mount("mqueue", "/tmp/pk_ipcmq/a", "mqueue", 0, P_NULL), 0);
    expect("starts_empty", dir_entries("/tmp/pk_ipcmq/a"), 2);
    i64 fd = syscall4(SYS_mq_open, name, O_CREAT | O_RDWR, 0600, 0);
    check("create", fd >= 0, fd);
    check("listed", p_dir_has("/tmp/pk_ipcmq/a", name), 0);
    struct a_stat st;
    char path[96];
    str_cpy(str_cpy(path, "/tmp/pk_ipcmq/a/"), name);
    expect("stat", a_stat_path(path, &st), 0);
    expect("mode_0600", st.st_mode, 0100600);

    p_read_file("/proc/sys/kernel/msgmni", buf, sizeof buf);
    outer_msgmni = (i64)p_num(buf);
    p_read_file("/proc/sys/fs/mqueue/msg_max", buf, sizeof buf);
    outer_msg_max = (i64)p_num(buf);

    p_run("inner_ns", CLONE_NEWIPC, inner);

    p_read_file("/proc/sys/kernel/msgmni", buf, sizeof buf);
    expect("outer_msgmni_kept", (i64)p_num(buf), outer_msgmni);
    p_read_file("/proc/sys/fs/mqueue/msg_max", buf, sizeof buf);
    expect("outer_msg_max_kept", (i64)p_num(buf), outer_msg_max);
    check("outer_queue_still", p_dir_has("/tmp/pk_ipcmq/a", name), 0);
    expect("unlink", syscall1(SYS_mq_unlink, name), 0);
    expect("listing_empty", dir_entries("/tmp/pk_ipcmq/a"), 2);
    expect("umount", syscall2(SYS_umount2, "/tmp/pk_ipcmq/a", 0), 0);
    a_rmdir("/tmp/pk_ipcmq/a");
    a_rmdir("/tmp/pk_ipcmq");
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    char n[24];
    str_cpy(str_cpy(name, "pk_mq"), a_utoa(n, (u64)syscall0(SYS_getpid)));
    p_run("ipcns_mqueue", CLONE_NEWIPC | CLONE_NEWNS, body);
    return finish();
}
