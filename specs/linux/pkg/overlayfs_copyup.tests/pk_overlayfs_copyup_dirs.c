/* Creating entries in the merged view puts them in the upper layer, and the
 * directories above them are copied up first, as empty directories with the
 * lower directory's mode and owner (siblings and other children are not
 * copied but still show through). New entries get the creator's ids and the
 * mode minus umask; in a set-group-ID directory the group of the directory
 * and, for a new directory, the set-group-ID bit. link() of a lower file
 * copies the file up and links the upper copy; with index=off a hard link
 * that exists in the lower layer is not preserved across copy-up (the other
 * name keeps the old content). Special files (fifo, device node, socket
 * path) of a lower layer show through with their type and device number.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovc_dirs"
#define M B "/m"
#define U B "/u"
#define L B "/l"
#define SYS_umask_ 95

static int body(void) {
    syscall1(SYS_umask_, 022);
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(L "/a/b/c");
    pm_mkdirs(L "/a/sibling");
    pm_mkdirs(L "/sg");
    pm_mkdirs(U);
    pm_mkdirs(B "/w");
    pm_mkdirs(M);
    syscall3(SYS_chmod, L "/a", 0701, 0);
    syscall3(SYS_chown, L "/a", 21, 22);
    syscall3(SYS_chmod, L "/a/b", 0702, 0);
    syscall3(SYS_chown, L "/a/b", 23, 24);
    syscall3(SYS_chmod, L "/a/b/c", 0703, 0);
    syscall3(SYS_chown, L "/a/b/c", 25, 26);
    pm_put(L, "/a/b/c/file", "deep", 0644);
    pm_put(L, "/a/sibling/s", "sib", 0644);
    syscall3(SYS_chmod, L "/sg", 02775, 0);
    syscall3(SYS_chown, L "/sg", 0, 50);
    pm_put(L, "/h1", "hardlinked", 0644);
    syscall3(SYS_link, L "/h1", L "/h2", 0);
    pm_put(L, "/lk", "linkme", 0644);
    pm_mknod(L "/fifo", S_IFIFO | 0644, 0, 0);
    pm_mknod(L "/chr", S_IFCHR | 0600, 4, 77);

    const char *opts = "lowerdir=" L ",upperdir=" U ",workdir=" B "/w,metacopy=off,redirect_dir=off,index=off";
    expect("mount", pm_mount("ov", M, "overlay", 0, opts), 0);
    struct a_stat st;

    /* the directory chain is copied, one directory per level */
    expect("deep_write", a_make_file(M "/a/b/c/file", "deeper", 0644), 0);
    expect("upper_a", a_stat_path(U "/a", &st), 0);
    expect("upper_a_mode", st.st_mode, S_IFDIR | 0701);
    expect("upper_a_uid", st.st_uid, 21);
    expect("upper_a_gid", st.st_gid, 22);
    expect("upper_b", a_stat_path(U "/a/b", &st), 0);
    expect("upper_b_mode", st.st_mode, S_IFDIR | 0702);
    expect("upper_b_uid", st.st_uid, 23);
    expect("upper_c", a_stat_path(U "/a/b/c", &st), 0);
    expect("upper_c_mode", st.st_mode, S_IFDIR | 0703);
    expect("upper_c_gid", st.st_gid, 26);
    pm_expect_list("upper_a_has_only_b", U "/a", "b");
    pm_expect_list("merged_a_has_both", M "/a", "b,sibling");
    pm_expect_file("sibling_still_visible", M "/a/sibling/s", "sib");
    expect("sibling_not_copied", pm_exists(U "/a/sibling"), 0);
    pm_expect_file("deep_upper", U "/a/b/c/file", "deeper");
    pm_expect_file("deep_lower", L "/a/b/c/file", "deep");

    /* creation in a lower-only directory */
    expect("mkdir_in_sibling", a_mkdir(M "/a/sibling/newdir", 0770), 0);
    expect("sibling_copied_for_mkdir", a_stat_path(U "/a/sibling", &st), 0);
    expect("newdir_mode_umask", pm_mode(M "/a/sibling/newdir"), S_IFDIR | 0750);
    expect("newfile", a_make_file(M "/a/sibling/created", "x", 0666), 0);
    expect("newfile_mode_umask", pm_mode(M "/a/sibling/created"), S_IFREG | 0644);
    expect("newfile_owner", a_stat_path(M "/a/sibling/created", &st), 0);
    expect("newfile_uid", st.st_uid, 0);
    expect("newfile_gid", st.st_gid, 0);
    expect("symlink_new", syscall3(SYS_symlink, "created", M "/a/sibling/sl", 0), 0);
    expect("symlink_type", pm_mode(U "/a/sibling/sl") & S_IFMT, S_IFLNK);
    expect("mknod_fifo_new", pm_mknod(M "/a/sibling/newfifo", S_IFIFO | 0600, 0, 0), 0);
    expect("fifo_type", pm_mode(U "/a/sibling/newfifo") & S_IFMT, S_IFIFO);
    pm_expect_list("sibling_listing", M "/a/sibling", "created,newdir,newfifo,s,sl");

    /* set-group-ID directory */
    expect("sg_file", a_make_file(M "/sg/f", "g", 0644), 0);
    expect("sg_file_stat", a_stat_path(M "/sg/f", &st), 0);
    expect("sg_file_gid", st.st_gid, 50);
    expect("sg_dir", a_mkdir(M "/sg/d", 0755), 0);
    expect("sg_dir_stat", a_stat_path(M "/sg/d", &st), 0);
    expect("sg_dir_gid", st.st_gid, 50);
    expect("sg_dir_setgid", st.st_mode & S_ISGID, S_ISGID);
    expect("sg_upper_dir_mode", pm_mode(U "/sg"), S_IFDIR | 02775);

    /* hard links */
    expect("h1_nlink_before", a_stat_path(M "/h1", &st), 0);
    expect("h1_nlink_two", st.st_nlink, 2);
    expect("h1_write", a_make_file(M "/h1", "changed", 0644), 0);
    pm_expect_file("h1_new_content", M "/h1", "changed");
    pm_expect_file("h2_old_content", M "/h2", "hardlinked");
    expect("h2_not_in_upper", pm_exists(U "/h2"), 0);
    expect("link_lower_file", syscall3(SYS_link, M "/lk", M "/lk2", 0), 0);
    expect("link_copied_up", pm_exists(U "/lk"), 1);
    expect("link_in_upper", pm_exists(U "/lk2"), 1);
    expect("link_stat", a_stat_path(M "/lk2", &st), 0);
    expect("link_nlink", st.st_nlink, 2);
    expect("link_write", a_make_file(M "/lk2", "via-lk2", 0644), 0);
    pm_expect_file("link_other_name_sees", M "/lk", "via-lk2");
    pm_expect_file("link_lower_untouched", L "/lk", "linkme");

    /* special files of a lower layer */
    expect("fifo_type_merged", pm_mode(M "/fifo"), S_IFIFO | 0644);
    expect("chr_stat", a_stat_path(M "/chr", &st), 0);
    expect("chr_type", st.st_mode, S_IFCHR | 0600);
    struct a_stat lst;
    expect("chr_lower_stat", a_stat_path(L "/chr", &lst), 0);
    expect("chr_rdev_same", st.st_rdev, lst.st_rdev);
    expect("chr_rdev_nonzero", st.st_rdev != 0, 1);
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
