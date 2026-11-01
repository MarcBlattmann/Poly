/* fadvise64 on descriptors of other kinds and access modes: a file open
 * O_RDONLY, O_WRONLY or O_WRONLY|O_APPEND, a directory, /dev/null,
 * /dev/zero, a memfd, a file in /dev/shm (tmpfs) and a Unix socket all
 * accept every advice (0). Read access is not needed. A negative len is
 * still EINVAL on all of them.
 */
#include "a_sys.h"

#define SYS_fadvise64 221
#define SYS_socket 41
#define SYS_memfd_create 319
#define AF_UNIX 1
#define SOCK_STREAM 1
#define POSIX_FADV_SEQUENTIAL 2
#define POSIX_FADV_WILLNEED 3
#define POSIX_FADV_DONTNEED 4

static i64 fadv(i64 fd, i64 off, i64 len, i64 adv) { return syscall4(SYS_fadvise64, fd, off, len, adv); }

static void all_advice(const char *name, i64 fd) {
    int ok = 1;
    i64 bad = 0;
    for (int adv = 0; adv <= 5; adv++) {
        i64 r = fadv(fd, 0, 4096, adv);
        if (r != 0) ok = 0, bad = r;
    }
    check(name, ok, bad);
}

int test_main(void) {
    const char *file = "/tmp/ld_fadvise64_kinds";
    const char *shm = "/dev/shm/ld_fadvise64_kinds";
    i64 fd = a_open(file, O_RDWR | O_CREAT | O_TRUNC, 0644);
    a_write(fd, "0123456789", 10);
    a_close(fd);

    i64 ro = a_open(file, O_RDONLY, 0);
    i64 wo = a_open(file, O_WRONLY, 0);
    i64 ap = a_open(file, O_WRONLY | O_APPEND, 0);
    i64 dir = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    i64 dn = a_open("/dev/null", O_RDWR, 0);
    i64 dz = a_open("/dev/zero", O_RDONLY, 0);
    i64 mfd = syscall2(SYS_memfd_create, "ld_fadvise64", 0);
    a_write(mfd, "memfd", 5);
    i64 sf = a_open(shm, O_RDWR | O_CREAT | O_TRUNC, 0600);
    a_write(sf, "shm", 3);
    i64 sock = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    check("all_open", ro >= 0 && wo >= 0 && ap >= 0 && dir >= 0 && dn >= 0 && dz >= 0 && mfd >= 0 && sf >= 0 && sock >= 0,
          0);

    all_advice("rdonly_file", ro);
    all_advice("wronly_file", wo);
    all_advice("append_file", ap);
    all_advice("directory", dir);
    all_advice("dev_null", dn);
    all_advice("dev_zero", dz);
    all_advice("memfd", mfd);
    all_advice("dev_shm_file", sf);
    all_advice("unix_socket", sock);

    expect("rdonly_negative_len", fadv(ro, 0, -1, POSIX_FADV_DONTNEED), -EINVAL);
    expect("directory_negative_len", fadv(dir, 0, -1, POSIX_FADV_WILLNEED), -EINVAL);
    expect("dev_null_negative_len", fadv(dn, 0, -1, POSIX_FADV_SEQUENTIAL), -EINVAL);
    expect("memfd_negative_len", fadv(mfd, 0, -1, POSIX_FADV_DONTNEED), -EINVAL);
    expect("socket_negative_len", fadv(sock, 0, -1, POSIX_FADV_DONTNEED), -EINVAL);
    expect("directory_bad_advice", fadv(dir, 0, 0, 6), -EINVAL);

    /* The memfd's data and offset survive DONTNEED. */
    expect("memfd_offset", syscall3(SYS_lseek, mfd, 0, SEEK_CUR), 5);
    char b[8];
    memset(b, 0, sizeof b);
    expect("memfd_pread", syscall4(SYS_pread64, mfd, b, sizeof b, 0), 5);
    check("memfd_data", mem_eq(b, "memfd", 5), 0);

    a_close(ro);
    a_close(wo);
    a_close(ap);
    a_close(dir);
    a_close(dn);
    a_close(dz);
    a_close(mfd);
    a_close(sf);
    a_close(sock);
    expect("unlink_shm", a_unlink(shm), 0);
    expect("unlink_file", a_unlink(file), 0);
    return finish();
}
