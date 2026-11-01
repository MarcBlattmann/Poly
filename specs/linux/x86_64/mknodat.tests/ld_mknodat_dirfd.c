/* mknodat(dirfd, path, mode, dev) is mknod with a relative path resolved
 * from the directory dirfd: an O_RDONLY or O_PATH directory descriptor,
 * or AT_FDCWD for the working directory. An absolute path ignores dirfd
 * (even an invalid one). dirfd is an int: bits 32..63 are ignored. ".."
 * from dirfd works (nothing confines the path). The node is the same as
 * mknod would make (type, mode & ~umask, owner).
 */
#include "a_sys.h"

#define SYS_mknodat 259
#define DIR "/tmp/ld_mknodat_dirfd"

static struct a_stat st;

static void cleanup(void) {
    static const char *names[] = {"a", "b", "c", "d", "e", "sub/up", "g", 0};
    char p[80];
    for (int i = 0; names[i]; i++) a_unlink(str_cat2(p, DIR "/", names[i]));
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    expect("mkdir_sub", a_mkdir(DIR "/sub", 0755), 0);
    i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
    expect("dirfd_3", d, 3);

    expect("relative_to_dirfd", syscall4(SYS_mknodat, d, "a", S_IFIFO | 0640, 0), 0);
    expect("a_mode", (a_stat_path(DIR "/a", &st), st.st_mode), S_IFIFO | 0640);
    expect("a_owner", st.st_uid, syscall0(SYS_geteuid));
    expect("regular_relative", syscall4(SYS_mknodat, d, "b", S_IFREG | 0666, 0), 0);
    expect("b_mode", (a_stat_path(DIR "/b", &st), st.st_mode), S_IFREG | 0644);

    i64 pd = a_open(DIR, O_PATH | O_DIRECTORY, 0);
    expect("opath_dirfd_4", pd, 4);
    expect("relative_to_opath_dirfd", syscall4(SYS_mknodat, pd, "c", S_IFIFO | 0600, 0), 0);
    expect("c_exists", a_stat_path(DIR "/c", &st), 0);

    expect("absolute_ignores_bad_dirfd", syscall4(SYS_mknodat, 99, DIR "/d", S_IFIFO | 0600, 0), 0);
    expect("d_exists", a_stat_path(DIR "/d", &st), 0);
    expect("dirfd_high_bits_ignored", syscall4(SYS_mknodat, (1L << 32) | d, "e", S_IFIFO | 0600, 0), 0);
    expect("e_exists", a_stat_path(DIR "/e", &st), 0);

    i64 sd = a_open(DIR "/sub", O_RDONLY | O_DIRECTORY, 0);
    expect("subdirfd_5", sd, 5);
    expect("dotdot_from_dirfd", syscall4(SYS_mknodat, sd, "../g", S_IFIFO | 0600, 0), 0);
    expect("g_in_parent", a_stat_path(DIR "/g", &st), 0);
    expect("sub_relative_nested", syscall4(SYS_mknodat, d, "sub/up", S_IFSOCK | 0600, 0), 0);
    expect("up_is_socket", (a_stat_path(DIR "/sub/up", &st), st.st_mode & S_IFMT), S_IFSOCK);

    expect("chdir", syscall1(SYS_chdir, DIR "/sub"), 0);
    expect("at_fdcwd_relative_eexist", syscall4(SYS_mknodat, AT_FDCWD, "up", S_IFIFO | 0600, 0), -EEXIST);
    expect("at_fdcwd_dotdot", syscall4(SYS_mknodat, AT_FDCWD, "../a", S_IFIFO | 0600, 0), -EEXIST);
    syscall1(SYS_chdir, "/");

    /* The directory may be renamed: dirfd still names it. */
    expect("rename_dir", syscall2(SYS_rename, DIR "/sub", DIR "/sub2"), 0);
    expect("dirfd_follows_rename", syscall4(SYS_mknodat, sd, "moved", S_IFIFO | 0600, 0), 0);
    expect("moved_exists", a_stat_path(DIR "/sub2/moved", &st), 0);
    a_unlink(DIR "/sub2/moved");
    syscall2(SYS_rename, DIR "/sub2", DIR "/sub");

    a_close(d);
    a_close(pd);
    a_close(sd);
    cleanup();
    return finish();
}
