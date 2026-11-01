/* symlink(2) creates a symbolic link whose contents are the target string,
 * stored as given and never checked: it may be relative, absolute,
 * missing, contain "..", spaces or bytes above 127, and be up to 4095
 * bytes. The link is S_IFLNK|0777 (the umask does not apply), owned by the
 * caller, st_size = target length; the parent's mtime changes. */
#include "a_sys.h"

#define D "/tmp/lb_symlink_basic"

static i64 symlink_(const char *t, const char *l) { return syscall2(SYS_symlink, t, l); }
static i64 readlink_(const char *p, char *b, i64 n) { return syscall3(SYS_readlink, p, b, n); }

static char target[4200], out[4200];

int test_main(void) {
    struct a_stat st, pst;
    char c[64];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "hello", 0644), 0);
    struct a_timespec ts[2] = {{1000000000, 0}, {1000000000, 0}};
    expect("set_dir_times", syscall4(SYS_utimensat, AT_FDCWD, D, ts, 0), 0);

    expect("relative", symlink_("f", D "/l"), 0);
    expect("lstat_l", a_lstat_path(D "/l", &st), 0);
    expect("mode_lnk_0777", st.st_mode, S_IFLNK | 0777);
    expect("size_1", st.st_size, 1);
    expect("nlink_1", st.st_nlink, 1);
    expect("uid", st.st_uid, syscall0(SYS_geteuid));
    expect("gid", st.st_gid, syscall0(SYS_getegid));
    expect("stat_follows", a_stat_path(D "/l", &st) == 0 && st.st_size == 5, 1);
    i64 fd = a_open(D "/l", O_RDONLY, 0);
    expect("open_through_link", a_read(fd, c, 64), 5);
    a_close(fd);
    expect("stat_dir", a_stat_path(D, &pst), 0);
    check("parent_mtime_changed", pst.st_mtime_sec > 1000000000, 0);

    /* umask 0777 does not matter */
    i64 old = syscall1(SYS_umask, 0777);
    expect("umask_link", symlink_("f", D "/lu"), 0);
    syscall1(SYS_umask, old);
    expect("umask_link_mode", a_lstat_path(D "/lu", &st) == 0 && st.st_mode == (S_IFLNK | 0777), 1);

    /* targets are stored verbatim */
    static const char *targets[] = {"/absolute/path/that/does/not/exist", "../../..", "a b  c", "x//y/./z/",
                                    "\xc3\xa9t\xc3\xa9", ".", "/", "-"};
    for (unsigned i = 0; i < sizeof targets / sizeof targets[0]; i++) {
        char name[64];
        str_cpy(str_cpy(name, D "/t"), "0");
        name[str_len(name) - 1] = (char)('0' + i);
        expect("store_target", symlink_(targets[i], name), 0);
        i64 n = readlink_(name, c, 64);
        expect("read_back_len", n, (i64)str_len(targets[i]));
        expect("read_back_bytes", mem_eq(c, targets[i], str_len(targets[i])), 1);
        expect("size_is_len", a_lstat_path(name, &st) == 0 && st.st_size == (i64)str_len(targets[i]), 1);
        expect("cleanup_t", a_unlink(name), 0);
    }

    /* the longest target: 4095 bytes */
    for (int i = 0; i < 4095; i++) target[i] = (char)('a' + i % 26);
    target[4095] = 0;
    expect("target_4095", symlink_(target, D "/long"), 0);
    expect("target_4095_size", a_lstat_path(D "/long", &st) == 0 ? st.st_size : -1, 4095);
    expect("target_4095_read", readlink_(D "/long", out, sizeof out), 4095);
    expect("target_4095_bytes", mem_eq(out, target, 4095), 1);
    expect("target_4095_follow", a_stat_path(D "/long", &st), -ENAMETOOLONG);

    /* links to directories, chains, dangling */
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("dirlink", symlink_("sub", D "/dl"), 0);
    expect("create_through_dirlink", a_make_file(D "/dl/inner", "x", 0644), 0);
    expect("inner_in_sub", a_lstat_path(D "/sub/inner", &st), 0);
    expect("chain", symlink_("l", D "/chain"), 0);
    expect("chain_resolves", a_stat_path(D "/chain", &st) == 0 && st.st_size == 5, 1);
    expect("dangling", symlink_("nothing/here", D "/dangling"), 0);
    expect("dangling_stat", a_stat_path(D "/dangling", &st), -ENOENT);
    /* relative targets are relative to the link's directory, not the cwd */
    expect("sub_link", symlink_("../f", D "/sub/up"), 0);
    expect("sub_link_resolves", a_stat_path(D "/sub/up", &st) == 0 && st.st_size == 5, 1);
    /* relative linkpath */
    expect("chdir", syscall1(SYS_chdir, D "/sub"), 0);
    expect("relative_linkpath", symlink_("inner", "rel"), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("rel_resolves", a_stat_path(D "/sub/rel", &st), 0);

    static const char *rm[] = {D "/sub/rel", D "/sub/up", D "/sub/inner", D "/dangling", D "/chain", D "/dl",
                               D "/long", D "/lu", D "/l", D "/f"};
    for (unsigned i = 0; i < sizeof rm / sizeof rm[0]; i++) expect("cleanup_unlink", a_unlink(rm[i]), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
