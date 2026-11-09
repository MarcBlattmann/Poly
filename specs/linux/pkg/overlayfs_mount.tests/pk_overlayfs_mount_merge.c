/* An overlay mount presents its layers as one tree: the upper layer, then the
 * lower layers from left to right in lowerdir=, the first layer that has a
 * name wins. Directories present in several layers are merged, readdir lists
 * every name once, and attributes of a merged file or directory (mode, owner,
 * size, contents) come from the layer that wins. A non-directory hides a
 * directory of the same name in a layer below; a directory hides a
 * non-directory below it (its content is not merged in). statfs reports the
 * overlay magic 0x794c7630.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovm_merge"
#define SYS_chown_ SYS_chown

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(B "/l1/d");
    pm_mkdirs(B "/l1/clash2");
    pm_mkdirs(B "/l2/d");
    pm_mkdirs(B "/l2/clash");
    pm_mkdirs(B "/l2/clash/inner");
    pm_mkdirs(B "/u/d");
    pm_mkdirs(B "/w");
    pm_mkdirs(B "/m");
    /* l1 (leftmost lower), l2, and the upper layer */
    pm_put(B, "/l1/a", "one", 0640);
    syscall3(SYS_chown_, B "/l1/a", 1000, 2000);
    pm_put(B, "/l2/a", "two-two", 0600);
    syscall3(SYS_chown_, B "/l2/a", 3, 4);
    pm_put(B, "/l2/b", "bee", 0644);
    pm_put(B, "/l1/only1", "o1", 0644);
    pm_put(B, "/l2/only2", "o22", 0644);
    pm_put(B, "/l1/c", "c-from-l1", 0644);
    pm_put(B, "/u/c", "c-from-upper", 0600);
    pm_put(B, "/u/u_only", "uu", 0644);
    pm_put(B, "/l1/d/x", "dx1", 0644);
    pm_put(B, "/l2/d/y", "dy2", 0644);
    pm_put(B, "/l2/d/x", "dx2", 0644);
    pm_put(B, "/u/d/z", "dz", 0644);
    syscall3(SYS_chmod, B "/l1/d", 0750, 0);
    syscall3(SYS_chown_, B "/l1/d", 5, 6);
    syscall3(SYS_chmod, B "/l2/d", 0700, 0);
    syscall3(SYS_chown_, B "/l2/d", 7, 8);
    syscall3(SYS_symlink, "a", B "/l2/lnk", 0);
    /* a file in l1 over a directory in l2; a directory in l1 over a file in l2 */
    pm_put(B, "/l1/clash", "file-wins", 0644);
    pm_put(B, "/l2/clash2", "file-below", 0644);

    const char *opts = "lowerdir=" B "/l1:" B "/l2,upperdir=" B "/u,workdir=" B "/w";
    expect("mount", pm_mount("ov", B "/m", "overlay", 0, opts), 0);

    struct a_statfs fs;
    expect("statfs", syscall2(SYS_statfs, B "/m", &fs), 0);
    expect("statfs_type", fs.f_type, OVERLAYFS_SUPER_MAGIC);

    pm_expect_list("readdir_root", B "/m", "a,b,c,clash,clash2,d,lnk,only1,only2,u_only");
    pm_expect_list("readdir_merged_dir", B "/m/d", "x,y,z");
    pm_expect_list("readdir_dir_over_file", B "/m/clash2", "");

    pm_expect_file("leftmost_lower_wins", B "/m/a", "one");
    pm_expect_file("only_in_second_lower", B "/m/b", "bee");
    pm_expect_file("upper_wins_over_lowers", B "/m/c", "c-from-upper");
    pm_expect_file("dir_x_from_leftmost", B "/m/d/x", "dx1");
    pm_expect_file("dir_y_from_second", B "/m/d/y", "dy2");
    pm_expect_file("dir_z_from_upper", B "/m/d/z", "dz");
    pm_expect_file("file_over_dir", B "/m/clash", "file-wins");
    expect("clash_is_a_file", pm_mode(B "/m/clash") & S_IFMT, S_IFREG);
    expect("dir_over_file_is_dir", pm_mode(B "/m/clash2") & S_IFMT, S_IFDIR);
    expect("lower_dir_under_file_hidden", pm_exists(B "/m/clash/inner"), 0);

    struct a_stat st;
    expect("stat_a", a_stat_path(B "/m/a", &st), 0);
    expect("a_mode", st.st_mode, S_IFREG | 0640);
    expect("a_uid", st.st_uid, 1000);
    expect("a_gid", st.st_gid, 2000);
    expect("a_size", st.st_size, 3);
    expect("stat_b", a_stat_path(B "/m/b", &st), 0);
    expect("b_size", st.st_size, 3);
    expect("stat_c", a_stat_path(B "/m/c", &st), 0);
    expect("c_mode", st.st_mode, S_IFREG | 0600);
    expect("c_size", st.st_size, 12);
    expect("stat_d", a_stat_path(B "/m/d", &st), 0);
    expect("d_mode_from_upper", st.st_mode, S_IFDIR | 0755);
    expect("d_uid_from_upper", st.st_uid, 0);

    expect("lnk_is_symlink", pm_mode(B "/m/lnk") & S_IFMT, S_IFLNK);
    char tgt[16];
    memset(tgt, 0, sizeof tgt);
    expect("readlink_len", syscall3(SYS_readlink, B "/m/lnk", tgt, sizeof tgt), 1);
    check("readlink_text", str_eq(tgt, "a"), 0);
    pm_expect_file("through_symlink", B "/m/lnk", "one");

    expect("missing_name", pm_exists(B "/m/nothing"), 0);
    expect("stat_missing", a_stat_path(B "/m/nothing", &st), -ENOENT);

    /* the lower layers themselves are unchanged */
    pm_expect_file("lower1_untouched", B "/l1/a", "one");
    pm_expect_file("lower2_untouched", B "/l2/a", "two-two");
    expect("umount", pm_umount(B "/m", 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
