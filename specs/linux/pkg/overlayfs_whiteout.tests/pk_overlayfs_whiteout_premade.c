/* Layers prepared before mounting. A character device 0:0 in the upper layer,
 * or in a lower layer above another one, hides the same name in the layers
 * below it, for files and for directories; it is not listed itself. A
 * directory with the extended attribute trusted.overlay.opaque = "y" hides
 * everything the layers below have at that name (the directory's own
 * entries are listed). A whiteout in the lowest layer is not listed either. The attribute
 * itself is hidden from get and list through the merged view. A character device with another number is an
 * ordinary device node. Re-adding a hidden name in the merged view works.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN; mknod; trusted.* attributes)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovw_pre"
#define M B "/m"
#define U B "/u"
#define L1 B "/l1"
#define L2 B "/l2"
#define L3 B "/l3"

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(L1 "/dwh");
    pm_mkdirs(L1 "/opq");
    pm_mkdirs(L2 "/dwh/inner");
    pm_mkdirs(L2 "/opq/inner");
    pm_mkdirs(L2 "/plaindir");
    pm_mkdirs(L3 "/opq2");
    pm_mkdirs(L3 "/deepdir");
    pm_mkdirs(U);
    pm_mkdirs(B "/w");
    pm_mkdirs(M);
    pm_put(L1, "/keep", "k1", 0644);
    pm_put(L2, "/hide_from_upper", "l2", 0644);
    pm_put(L2, "/hide_from_l1", "l2b", 0644);
    pm_put(L2, "/dwh/file", "x", 0644);
    pm_put(L2, "/opq/old", "o", 0644);
    pm_put(L2, "/opq/inner/f", "f", 0644);
    pm_put(L2, "/plaindir/p", "p", 0644);
    pm_put(L3, "/opq2/under", "u", 0644);
    pm_put(L3, "/deepdir/f", "df", 0644);
    pm_put(L3, "/lowest_wh_target", "t", 0644);

    /* whiteouts: upper hides l2, l1 hides l2, l1 hides a directory of l2 */
    expect("wh_in_upper", pm_mknod(U "/hide_from_upper", S_IFCHR, 0, 0), 0);
    expect("wh_in_l1", pm_mknod(L1 "/hide_from_l1", S_IFCHR, 0, 0), 0);
    expect("wh_dir_in_l1", pm_mknod(L1 "/plaindir", S_IFCHR, 0, 0), 0);
    /* a whiteout in the lowest layer is a plain node */
    expect("wh_in_lowest", pm_mknod(L3 "/lowest_wh", S_IFCHR, 0, 0), 0);
    /* a char device with another number is not a whiteout */
    expect("real_chardev_in_l1", pm_mknod(L1 "/notwh", S_IFCHR | 0600, 1, 5), 0);
    expect("wh_hides_chardev_name_in_l2", pm_mknod(L2 "/notwh", S_IFCHR, 0, 0), 0);
    /* a whiteout over a whole three-layer name */
    expect("wh_deepdir_in_l2", pm_mknod(L2 "/deepdir", S_IFCHR, 0, 0), 0);
    /* opaque directories */
    expect("opq_attr", syscall5(SYS_lsetxattr, L1 "/opq", "trusted.overlay.opaque", "y", 1, 0), 0);
    expect("opq2_in_l2", a_mkdir(L2 "/opq2", 0755), 0);
    expect("opq2_attr", syscall5(SYS_lsetxattr, L2 "/opq2", "trusted.overlay.opaque", "y", 1, 0), 0);
    pm_put(L2, "/opq2/new", "n", 0644);
    /* a directory in the upper layer over a lower directory: dwh is whited out partially */
    expect("wh_in_dwh", pm_mknod(L1 "/dwh/file", S_IFCHR, 0, 0), 0);

    const char *opts = "lowerdir=" L1 ":" L2 ":" L3 ",upperdir=" U ",workdir=" B "/w,index=off,metacopy=off";
    expect("mount", pm_mount("ov", M, "overlay", 0, opts), 0);

    pm_expect_list("root_listing", M, "dwh,keep,lowest_wh_target,notwh,opq,opq2");
    expect("hidden_by_upper", pm_exists(M "/hide_from_upper"), 0);
    expect("hidden_open", a_open(M "/hide_from_upper", O_RDONLY, 0), -ENOENT);
    expect("hidden_by_l1", pm_exists(M "/hide_from_l1"), 0);
    expect("hidden_dir_by_l1", pm_exists(M "/plaindir"), 0);
    expect("hidden_dir_child", pm_exists(M "/plaindir/p"), 0);
    expect("hidden_deepdir", pm_exists(M "/deepdir"), 0);
    expect("hidden_deep_child", pm_exists(M "/deepdir/f"), 0);
    expect("whiteout_name_not_listed", pm_exists(M "/lowest_wh_target"), 1);

    /* a whiteout in the lowest layer hides nothing but is not listed or visible either */
    struct a_stat st;
    expect("lowest_wh_invisible", pm_exists(M "/lowest_wh"), 0);
    expect("lowest_wh_enoent", a_stat_path(M "/lowest_wh", &st), -ENOENT);

    /* a real device node of l1 hides the whiteout of l2 below it? it is the top entry */
    expect("notwh_stat", a_stat_path(M "/notwh", &st), 0);
    expect("notwh_type", st.st_mode & S_IFMT, S_IFCHR);
    expect("notwh_rdev", st.st_rdev != 0, 1);

    /* opaque directories */
    pm_expect_list("opq_listing", M "/opq", "");
    expect("opq_hides_old", pm_exists(M "/opq/old"), 0);
    expect("opq_hides_inner", pm_exists(M "/opq/inner"), 0);
    expect("opq_mode", pm_mode(M "/opq") & S_IFMT, S_IFDIR);
    pm_expect_list("opq2_listing", M "/opq2", "new");
    expect("opq2_hides_l3", pm_exists(M "/opq2/under"), 0);
    pm_expect_file("opq2_new_visible", M "/opq2/new", "n");
    char v[8];
    expect("attr_hidden_get", syscall4(SYS_lgetxattr, M "/opq", "trusted.overlay.opaque", v, sizeof v), -ENODATA);
    char list[128];
    i64 n = syscall3(SYS_llistxattr, M "/opq", list, sizeof list);
    expect("attr_hidden_list", n, 0);

    /* dwh: l1's dwh hides the whiteout'ed file of l2 only */
    pm_expect_list("dwh_listing", M "/dwh", "inner");
    expect("dwh_file_hidden", pm_exists(M "/dwh/file"), 0);

    /* hidden names can be created again */
    expect("recreate_hidden", a_make_file(M "/hide_from_upper", "again", 0644), 0);
    pm_expect_file("recreated", M "/hide_from_upper", "again");
    pm_expect_file("recreated_in_upper", U "/hide_from_upper", "again");
    expect("mkdir_hidden_dir", a_mkdir(M "/plaindir", 0755), 0);
    pm_expect_list("mkdir_hidden_dir_empty", M "/plaindir", "");
    expect("umount", pm_umount(M, 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
