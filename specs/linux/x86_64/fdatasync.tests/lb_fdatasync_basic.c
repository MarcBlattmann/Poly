/* fdatasync(2) flushes a file's data to its storage and returns 0, for
 * any descriptor of a file or directory (read-only included); it changes
 * no visible state: contents, size, offset and times stay. */
#include "a_sys.h"

#define D "/tmp/lb_fdatasync_basic"

static i64 fdatasync_(i64 fd) { return syscall1(SYS_fdatasync, fd); }

int test_main(void) {
    struct a_stat st, st2;
    char c[16];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT, 0644);
    check("open", fd >= 0, fd);
    expect("empty_file", fdatasync_(fd), 0);
    expect("write", a_write(fd, "persist", 7), 7);
    expect("fstat0", syscall2(SYS_fstat, fd, &st), 0);
    expect("after_write", fdatasync_(fd), 0);
    expect("fstat1", syscall2(SYS_fstat, fd, &st2), 0);
    expect("stat_identical", mem_eq(&st, &st2, sizeof st), 1);
    expect("offset_kept", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 7);
    expect("contents", syscall4(SYS_pread64, fd, c, 16, 0) == 7 && mem_eq(c, "persist", 7), 1);
    expect("twice", fdatasync_(fd), 0);

    i64 ro = a_open(D "/f", O_RDONLY, 0);
    expect("rdonly_fd", fdatasync_(ro), 0);
    i64 wo = a_open(D "/f", O_WRONLY | O_APPEND, 0);
    expect("append_fd", fdatasync_(wo), 0);
    i64 dir = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("directory_fd", fdatasync_(dir), 0);
    i64 dup = syscall1(SYS_dup, fd);
    expect("dup_fd", fdatasync_(dup), 0);
    /* a file whose name is gone */
    expect("unlink", a_unlink(D "/f"), 0);
    expect("unlinked_file", fdatasync_(fd), 0);
    /* after ftruncate */
    expect("ftruncate", syscall2(SYS_ftruncate, fd, 1 << 20), 0);
    expect("after_ftruncate", fdatasync_(fd), 0);
    expect("size_after", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 1 << 20);
    /* the root directory and /dev/null's directory */
    i64 root = a_open("/", O_RDONLY | O_DIRECTORY, 0);
    expect("root_dir", fdatasync_(root), 0);

    a_close(root);
    a_close(dup);
    a_close(dir);
    a_close(wo);
    a_close(ro);
    a_close(fd);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
