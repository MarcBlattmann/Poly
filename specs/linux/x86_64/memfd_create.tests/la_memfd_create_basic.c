/* memfd_create(name, 0) returns the lowest free descriptor for a new,
 * empty, anonymous regular file: open O_RDWR | O_LARGEFILE, no
 * FD_CLOEXEC, mode 0777 whatever the umask, size 0, link count 0,
 * owned by the caller's euid/egid, st_blksize 4096. It can be written,
 * read back, seeked and truncated like a file. Two calls with the same
 * name give two independent files.
 */
#include "a_sys.h"

#define SYS_memfd_create 319

static i64 memfd(const char *name, u64 flags) { return syscall2(SYS_memfd_create, name, flags); }

int test_main(void) {
    i64 fd = memfd("basic", 0);
    expect("returns_fd_3", fd, 3);
    expect("getfl_rdwr_largefile", syscall2(SYS_fcntl, fd, F_GETFL), O_RDWR | O_LARGEFILE);
    expect("getfd_no_cloexec", syscall2(SYS_fcntl, fd, F_GETFD), 0);

    struct a_stat st;
    memset(&st, 0xff, sizeof st);
    expect("fstat_ok", syscall2(SYS_fstat, fd, &st), 0);
    expect("mode_reg_0777", st.st_mode, S_IFREG | 0777);
    expect("size_0", st.st_size, 0);
    expect("nlink_0", (i64)st.st_nlink, 0);
    expect("uid_is_euid", st.st_uid, syscall0(SYS_geteuid));
    expect("gid_is_egid", st.st_gid, syscall0(SYS_getegid));
    expect("blksize_4096", st.st_blksize, 4096);
    expect("blocks_0", st.st_blocks, 0);

    expect("write_5", a_write(fd, "hello", 5), 5);
    expect("lseek_cur_5", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 5);
    char buf[16];
    memset(buf, 0, sizeof buf);
    expect("pread_5", syscall4(SYS_pread64, fd, buf, sizeof buf, 0), 5);
    check("pread_data", mem_eq(buf, "hello", 5), 0);
    expect("ftruncate_grow", syscall2(SYS_ftruncate, fd, 10000), 0);
    syscall2(SYS_fstat, fd, &st);
    expect("size_after_truncate", st.st_size, 10000);
    expect("lseek_end", syscall3(SYS_lseek, fd, 0, SEEK_END), 10000);
    memset(buf, 0x55, sizeof buf);
    expect("pread_hole", syscall4(SYS_pread64, fd, buf, 4, 6000), 4);
    check("hole_reads_zero", buf[0] == 0 && buf[3] == 0, buf[0]);
    expect("ftruncate_shrink", syscall2(SYS_ftruncate, fd, 2), 0);
    expect("pread_after_shrink", syscall4(SYS_pread64, fd, buf, sizeof buf, 0), 2);

    /* Same name: an independent file. */
    i64 fd2 = memfd("basic", 0);
    expect("second_fd_4", fd2, 4);
    struct a_stat st2;
    syscall2(SYS_fstat, fd2, &st2);
    check("different_inode", st2.st_ino != st.st_ino, 0);
    expect("second_size_0", st2.st_size, 0);

    /* The umask does not apply. */
    syscall1(SYS_umask, 077);
    i64 fd3 = memfd("umask", 0);
    syscall2(SYS_fstat, fd3, &st);
    expect("umask_ignored", st.st_mode, S_IFREG | 0777);
    syscall1(SYS_umask, 022);

    /* fchmod works on it. */
    expect("fchmod_ok", syscall2(SYS_fchmod, fd3, 0600), 0);
    syscall2(SYS_fstat, fd3, &st);
    expect("fchmod_mode", st.st_mode, S_IFREG | 0600);

    expect("close", a_close(fd), 0);
    expect("close2", a_close(fd2), 0);
    expect("close3", a_close(fd3), 0);
    return finish();
}
