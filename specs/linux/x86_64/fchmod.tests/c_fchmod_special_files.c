/* fchmod also works on descriptors of files without a regular data
 * store: a pipe (both ends share one inode), a named FIFO, a UNIX
 * socket, a memfd and a file on tmpfs (/dev/shm). The type bits stay. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_socket 41
#define SYS_mknodat 259
#define SYS_memfd_create 319
/* linux/socket.h, linux/net.h */
#define AF_UNIX 1
#define SOCK_STREAM 1

#define FIFO "/tmp/c_fchmod_special_fifo"
#define SHM "/dev/shm/c_fchmod_special_shm"

static i64 fmode(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    int p[2];
    expect("pipe", syscall2(SYS_pipe2, p, 0), 0);
    expect("fchmod_pipe_read_end", syscall2(SYS_fchmod, p[0], 0640), 0);
    expect("pipe_read_end_mode", fmode(p[0]), S_IFIFO | 0640);
    expect("pipe_write_end_same_inode", fmode(p[1]), S_IFIFO | 0640);
    expect("fchmod_pipe_write_end", syscall2(SYS_fchmod, p[1], 0), 0);
    expect("pipe_mode_0", fmode(p[0]), S_IFIFO);
    a_close(p[0]);
    a_close(p[1]);

    a_unlink(FIFO);
    expect("mkfifo", syscall4(SYS_mknodat, AT_FDCWD, FIFO, S_IFIFO | 0600, 0), 0);
    i64 f = a_open(FIFO, O_RDWR, 0);
    check("open_fifo", f >= 0, f);
    expect("fchmod_fifo", syscall2(SYS_fchmod, f, 0604), 0);
    expect("fifo_mode", fmode(f), S_IFIFO | 0604);
    struct a_stat st;
    expect("stat_fifo", a_stat_path(FIFO, &st), 0);
    expect("fifo_path_mode", st.st_mode, S_IFIFO | 0604);
    a_close(f);
    expect("unlink_fifo", a_unlink(FIFO), 0);

    i64 s = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    check("socket", s >= 0, s);
    expect("fchmod_socket", syscall2(SYS_fchmod, s, 0600), 0);
    expect("socket_mode", fmode(s), S_IFSOCK | 0600);
    a_close(s);

    i64 m = syscall2(SYS_memfd_create, "c_fchmod", 0);
    check("memfd", m >= 0, m);
    expect("fchmod_memfd", syscall2(SYS_fchmod, m, 0640), 0);
    expect("memfd_mode", fmode(m), S_IFREG | 0640);
    a_close(m);

    a_unlink(SHM);
    i64 t = a_open(SHM, O_RDWR | O_CREAT | O_EXCL, 0600);
    check("open_shm", t >= 0, t);
    expect("fchmod_shm", syscall2(SYS_fchmod, t, 07755), 0);
    expect("shm_mode", fmode(t), S_IFREG | 07755);
    a_close(t);
    expect("unlink_shm", a_unlink(SHM), 0);
    return finish();
}
