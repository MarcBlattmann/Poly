/* mknod: success updates the parent directory's mtime and ctime (its link
 * count stays); the new node's atime, mtime and ctime are equal and not
 * earlier than the parent's old mtime. A failed call changes nothing. */
#include "a_sys.h"

#define SYS_mknod 133
#define D "/tmp/a_mknod_times"

static i64 mk(const char *p, i64 mode, i64 dev) { return syscall3(SYS_mknod, p, mode, dev); }

static i64 ns(u64 s, u64 n) { return (i64)s * 1000000000L + (i64)n; }

int test_main(void) {
    struct a_stat before, after, node, again;
    syscall1(SYS_umask, 022);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("stat_before", a_stat_path(D, &before), 0);
    a_spin_ticks(150000000UL); /* about 50 ms at 3 GHz */

    expect("mkfifo", mk(D "/p", S_IFIFO | 0644, 0), 0);
    expect("stat_after", a_stat_path(D, &after), 0);
    expect("stat_node", a_lstat_path(D "/p", &node), 0);
    check("parent_mtime_later", ns(after.st_mtime_sec, after.st_mtime_nsec) > ns(before.st_mtime_sec, before.st_mtime_nsec), 0);
    check("parent_ctime_later", ns(after.st_ctime_sec, after.st_ctime_nsec) > ns(before.st_ctime_sec, before.st_ctime_nsec), 0);
    expect("parent_nlink_same", (i64)after.st_nlink, (i64)before.st_nlink);
    check("node_atime_eq_mtime", ns(node.st_atime_sec, node.st_atime_nsec) == ns(node.st_mtime_sec, node.st_mtime_nsec), 0);
    check("node_ctime_eq_mtime", ns(node.st_ctime_sec, node.st_ctime_nsec) == ns(node.st_mtime_sec, node.st_mtime_nsec), 0);
    check("node_mtime_after_old_parent", ns(node.st_mtime_sec, node.st_mtime_nsec) > ns(before.st_mtime_sec, before.st_mtime_nsec), 0);
    check("node_mtime_not_after_parent", ns(node.st_mtime_sec, node.st_mtime_nsec) <= ns(after.st_mtime_sec, after.st_mtime_nsec), 0);

    a_spin_ticks(150000000UL);
    expect("fail_eexist", mk(D "/p", S_IFIFO | 0644, 0), -EEXIST);
    expect("fail_einval", mk(D "/x", 0170000, 0), -EINVAL);
    expect("fail_eperm_dir", mk(D "/x", S_IFDIR | 0755, 0), -EPERM);
    expect("stat_again", a_stat_path(D, &again), 0);
    check("fail_parent_mtime_same", ns(again.st_mtime_sec, again.st_mtime_nsec) == ns(after.st_mtime_sec, after.st_mtime_nsec), 0);
    check("fail_parent_ctime_same", ns(again.st_ctime_sec, again.st_ctime_nsec) == ns(after.st_ctime_sec, after.st_ctime_nsec), 0);
    struct a_stat node2;
    expect("stat_node_again", a_lstat_path(D "/p", &node2), 0);
    check("fail_node_ctime_same", ns(node2.st_ctime_sec, node2.st_ctime_nsec) == ns(node.st_ctime_sec, node.st_ctime_nsec), 0);

    expect("cleanup_p", a_unlink(D "/p"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
