/* sync_file_range on descriptors of other kinds: a regular file open
 * O_RDONLY or O_WRONLY, a directory, a memfd and a file in /dev/shm
 * (tmpfs) all return 0 (no write access needed). Descriptors that are not
 * open or were opened with O_PATH are EBADF (9). A pipe (either end), a
 * FIFO, a Unix socket and the character device /dev/null are ESPIPE (29).
 * Order: EBADF, then argument errors (EINVAL: flags, offset, nbytes),
 * then ESPIPE (a pipe with bad flags or a negative offset is EINVAL).
 */
#include "a_sys.h"

#define SYS_sync_file_range 277
#define SYS_socket 41
#define SYS_mknodat 259
#define SYS_memfd_create 319
#define ALL 7

static i64 sfr(i64 fd, i64 off, i64 n, u64 flags) { return syscall4(SYS_sync_file_range, fd, off, n, flags); }

int test_main(void) {
    const char *file = "/tmp/ld_sync_file_range_kinds";
    const char *fifo = "/tmp/ld_sync_file_range_kinds_fifo";
    const char *shm = "/dev/shm/ld_sync_file_range_kinds";
    i64 fd = a_open(file, O_RDWR | O_CREAT | O_TRUNC, 0644);
    a_write(fd, "kinds", 5);

    i64 ro = a_open(file, O_RDONLY, 0);
    expect("rdonly_file", sfr(ro, 0, 0, ALL), 0);
    i64 wo = a_open(file, O_WRONLY, 0);
    expect("wronly_file", sfr(wo, 0, 0, ALL), 0);
    i64 dir = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    expect("directory", sfr(dir, 0, 0, ALL), 0);
    i64 mfd = syscall2(SYS_memfd_create, "ld_sfr", 0);
    a_write(mfd, "m", 1);
    expect("memfd", sfr(mfd, 0, 0, ALL), 0);
    i64 sf = a_open(shm, O_RDWR | O_CREAT | O_TRUNC, 0600);
    a_write(sf, "s", 1);
    expect("dev_shm_file", sfr(sf, 0, 0, ALL), 0);

    expect("closed", sfr(999, 0, 0, ALL), -EBADF);
    expect("minus1", sfr(-1, 0, 0, ALL), -EBADF);
    i64 pa = a_open(file, O_PATH, 0);
    expect("o_path", sfr(pa, 0, 0, ALL), -EBADF);

    int p[2];
    syscall2(SYS_pipe2, p, 0);
    expect("pipe_read_end", sfr(p[0], 0, 0, ALL), -ESPIPE);
    expect("pipe_write_end", sfr(p[1], 0, 0, 0), -ESPIPE);
    syscall4(SYS_mknodat, AT_FDCWD, fifo, S_IFIFO | 0600, 0);
    i64 ff = a_open(fifo, O_RDWR, 0);
    expect("fifo", sfr(ff, 0, 0, ALL), -ESPIPE);
    i64 sock = syscall3(SYS_socket, 1, 1, 0);
    expect("socket", sfr(sock, 0, 0, ALL), -ESPIPE);
    i64 dn = a_open("/dev/null", O_RDWR, 0);
    expect("dev_null", sfr(dn, 0, 0, ALL), -ESPIPE);

    /* Order. */
    expect("o_path_bad_flags", sfr(pa, 0, 0, 8), -EBADF);
    expect("pipe_bad_flags", sfr(p[0], 0, 0, 8), -EINVAL);
    expect("pipe_negative_offset", sfr(p[0], -1, 0, 0), -EINVAL);
    expect("pipe_negative_nbytes", sfr(p[0], 0, -1, 0), -EINVAL);
    expect("socket_bad_flags", sfr(sock, 0, 0, 0x10), -EINVAL);

    a_close(dn);
    a_close(sock);
    a_close(ff);
    a_close(p[0]);
    a_close(p[1]);
    a_close(pa);
    a_close(sf);
    a_close(mfd);
    a_close(dir);
    a_close(wo);
    a_close(ro);
    a_close(fd);
    expect("unlink_shm", a_unlink(shm), 0);
    expect("unlink_fifo", a_unlink(fifo), 0);
    expect("unlink_file", a_unlink(file), 0);
    return finish();
}
