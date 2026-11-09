/* More ways to trigger or avoid a copy-up. lchown of a lower symlink copies
 * the symlink itself (target text kept, owner changed); fchmod and fchown on
 * a descriptor opened O_RDONLY copy the file up; a MAP_SHARED writable
 * mapping of a file opened O_RDWR writes into the upper copy (lower file
 * unchanged); reading through MAP_PRIVATE does not copy; a file unlinked
 * while open stays readable and leaves a whiteout; stat, access, readlink,
 * getdents and exec-bit checks never copy; unlinking an upper-only name that
 * was copied up earlier leaves a whiteout because the lower layer still has
 * the name.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovc_misc"
#define M B "/m"
#define U B "/u"
#define L B "/l"
#define SYS_lchown_ 94
#define SYS_fchmod_ SYS_fchmod
#define SYS_fchown_ SYS_fchown

static int whiteout(const char *p) {
    struct a_stat st;
    return a_lstat_path(p, &st) == 0 && (st.st_mode & S_IFMT) == S_IFCHR && st.st_rdev == 0;
}

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(L);
    pm_mkdirs(U);
    pm_mkdirs(B "/w");
    pm_mkdirs(M);
    pm_put(L, "/target", "tgt", 0644);
    syscall3(SYS_symlink, "target", L "/link", 0);
    pm_put(L, "/fm", "fchmod-file", 0644);
    pm_put(L, "/fo", "fchown-file", 0644);
    pm_put(L, "/mm", "0123456789", 0644);
    pm_put(L, "/priv", "private-map", 0644);
    pm_put(L, "/open_unlink", "still-here", 0644);
    pm_put(L, "/x", "x", 0755);
    pm_put(L, "/gone_later", "g", 0644);

    const char *opts = "lowerdir=" L ",upperdir=" U ",workdir=" B "/w,metacopy=off,redirect_dir=off,index=off";
    expect("mount", pm_mount("ov", M, "overlay", 0, opts), 0);
    struct a_stat st;

    /* reads and lookups do not copy */
    expect("stat", a_stat_path(M "/x", &st), 0);
    expect("access_x", syscall2(SYS_access, M "/x", X_OK), 0);
    char tb[16];
    memset(tb, 0, sizeof tb);
    expect("readlink", syscall3(SYS_readlink, M "/link", tb, sizeof tb), 6);
    check("readlink_text", str_eq(tb, "target"), 0);
    pm_expect_list("readdir", M, "fm,fo,gone_later,link,mm,open_unlink,priv,target,x");
    expect("nothing_copied", pm_exists(U "/x") || pm_exists(U "/link"), 0);
    pm_expect_list("upper_empty", U, "");

    /* symlink */
    expect("lchown_symlink", syscall3(SYS_lchown_, M "/link", 33, 44), 0);
    expect("symlink_in_upper", a_lstat_path(U "/link", &st), 0);
    expect("symlink_upper_type", st.st_mode & S_IFMT, S_IFLNK);
    expect("symlink_upper_uid", st.st_uid, 33);
    expect("symlink_upper_gid", st.st_gid, 44);
    memset(tb, 0, sizeof tb);
    expect("symlink_upper_target_len", syscall3(SYS_readlink, U "/link", tb, sizeof tb), 6);
    check("symlink_upper_target", str_eq(tb, "target"), 0);
    expect("symlink_target_file_not_copied", pm_exists(U "/target"), 0);
    expect("symlink_lower_owner", a_lstat_path(L "/link", &st), 0);
    expect("symlink_lower_uid", st.st_uid, 0);
    pm_expect_file("symlink_still_resolves", M "/link", "tgt");

    /* descriptor based changes on an O_RDONLY descriptor */
    i64 fd = a_open(M "/fm", O_RDONLY, 0);
    check("open_fm", fd >= 0, fd);
    expect("fm_not_yet", pm_exists(U "/fm"), 0);
    expect("fchmod", syscall2(SYS_fchmod_, fd, 0600), 0);
    expect("fchmod_copied", pm_mode(U "/fm"), S_IFREG | 0600);
    char b[32];
    memset(b, 0, sizeof b);
    expect("fchmod_read_after", syscall4(SYS_pread64, fd, b, 11, 0), 11);
    check("fchmod_read_content", str_eq(b, "fchmod-file"), 0);
    a_close(fd);
    expect("fchmod_lower", pm_mode(L "/fm"), S_IFREG | 0644);
    fd = a_open(M "/fo", O_RDONLY, 0);
    expect("fchown", syscall3(SYS_fchown_, fd, 55, 66), 0);
    a_close(fd);
    expect("fchown_upper", a_stat_path(U "/fo", &st), 0);
    expect("fchown_upper_uid", st.st_uid, 55);
    expect("fchown_upper_gid", st.st_gid, 66);
    pm_expect_file("fchown_data", U "/fo", "fchown-file");

    /* mmap */
    fd = a_open(M "/priv", O_RDONLY, 0);
    i64 p = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE, fd, 0);
    check("mmap_private", p > 0 || p < -4096, p);
    check("mmap_private_content", p > 0 && ((char *)p)[0] == 'p' && ((char *)p)[1] == 'r', 0);
    syscall2(SYS_munmap, p, A_PAGE);
    a_close(fd);
    expect("private_read_not_copied", pm_exists(U "/priv"), 0);
    fd = a_open(M "/mm", O_RDWR, 0);
    check("open_mm", fd >= 0, fd);
    p = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("mmap_shared", p > 0, p);
    if (p > 0) {
        ((char *)p)[2] = 'X';
        ((char *)p)[3] = 'Y';
        expect("msync", syscall3(SYS_msync, p, A_PAGE, MS_SYNC), 0);
        syscall2(SYS_munmap, p, A_PAGE);
    }
    a_close(fd);
    pm_expect_file("mmap_upper", U "/mm", "01XY456789");
    pm_expect_file("mmap_merged", M "/mm", "01XY456789");
    pm_expect_file("mmap_lower", L "/mm", "0123456789");

    /* unlink while open */
    fd = a_open(M "/open_unlink", O_RDONLY, 0);
    expect("unlink_open", a_unlink(M "/open_unlink"), 0);
    memset(b, 0, sizeof b);
    expect("read_after_unlink", a_read(fd, b, 10), 10);
    check("read_after_unlink_content", str_eq(b, "still-here"), 0);
    a_close(fd);
    expect("unlink_whiteout", whiteout(U "/open_unlink"), 1);
    expect("unlink_gone", pm_exists(M "/open_unlink"), 0);

    /* a copied-up name still needs a whiteout when removed */
    expect("copy_gone_later", a_make_file(M "/gone_later", "new", 0644), 0);
    pm_expect_file("copy_in_upper", U "/gone_later", "new");
    expect("unlink_copied", a_unlink(M "/gone_later"), 0);
    expect("copied_whiteout", whiteout(U "/gone_later"), 1);
    expect("copied_hidden", pm_exists(M "/gone_later"), 0);
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
