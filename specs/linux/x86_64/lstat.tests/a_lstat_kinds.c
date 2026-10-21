/* lstat: the file type of directories, FIFOs, sockets and character
 * devices; the root directory and its "..". */
#include "a_sys.h"

#define D "/tmp/a_lstat_kinds"
#define SYS_socket 41
#define SYS_bind 49
#define SYS_mknodat 259
#define AF_UNIX 1
#define SOCK_STREAM 1

static i64 lstat(const char *p, struct a_stat *st) { return syscall2(SYS_lstat, p, st); }

struct sockaddr_un {
    u16 sun_family;
    char sun_path[108];
};

int test_main(void) {
    struct a_stat st, root;
    expect("setup_mkdir", a_mkdir(D, 0750), 0);
    expect("dir_ok", lstat(D, &st), 0);
    expect("dir_mode", st.st_mode, S_IFDIR | 0750);
    expect("dir_nlink", st.st_nlink, 2);
    expect("dir_rdev", st.st_rdev, 0);
    expect("dir_trailing_slash", lstat(D "/", &st), 0);
    expect("dir_slash_mode", st.st_mode, S_IFDIR | 0750);

    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0640, 0), 0);
    expect("fifo_ok", lstat(D "/fifo", &st), 0);
    expect("fifo_mode", st.st_mode, S_IFIFO | 0640);
    expect("fifo_size", st.st_size, 0);
    expect("fifo_nlink", st.st_nlink, 1);
    expect("fifo_slash_enotdir", lstat(D "/fifo/", &st), -ENOTDIR);

    i64 s = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    check("setup_socket", s >= 0, s);
    struct sockaddr_un sa;
    memset(&sa, 0, sizeof sa);
    sa.sun_family = AF_UNIX;
    str_cpy(sa.sun_path, D "/sock");
    expect("setup_bind", syscall3(SYS_bind, s, &sa, sizeof sa), 0);
    expect("sock_ok", lstat(D "/sock", &st), 0);
    expect("sock_type", st.st_mode & S_IFMT, S_IFSOCK);
    expect("sock_perm_umask", st.st_mode & 07777, 0755);
    expect("sock_size", st.st_size, 0);
    a_close(s);
    expect("sock_after_close", lstat(D "/sock", &st), 0);
    expect("sock_still_socket", st.st_mode & S_IFMT, S_IFSOCK);

    expect("devnull_ok", lstat("/dev/null", &st), 0);
    expect("devnull_mode", st.st_mode, S_IFCHR | 0666);
    expect("devnull_rdev", st.st_rdev, (1 << 8) | 3);
    expect("devnull_size", st.st_size, 0);

    expect("root_ok", lstat("/", &root), 0);
    expect("root_type", root.st_mode & S_IFMT, S_IFDIR);
    expect("root_dotdot", lstat("/..", &st), 0);
    check("root_dotdot_is_root", st.st_ino == root.st_ino && st.st_dev == root.st_dev, 0);
    expect("root_many_dotdot", lstat("/../../..", &st), 0);
    check("root_many_dotdot_is_root", st.st_ino == root.st_ino, 0);
    expect("root_double_slash", lstat("//", &st), 0);
    check("root_double_slash_is_root", st.st_ino == root.st_ino, 0);

    expect("cleanup_sock", a_unlink(D "/sock"), 0);
    expect("cleanup_fifo", a_unlink(D "/fifo"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
