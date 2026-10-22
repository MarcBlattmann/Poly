/* unlink of a file that is mapped: the mapping keeps working, and shared
 * writes through it are seen through a descriptor, after the name is
 * gone and after the descriptor used to map it is closed. */
#include "a_sys.h"

#define D "/tmp/c_unlink_mmap"

static i64 u(const char *p) { return syscall1(SYS_unlink, p); }

int test_main(void) {
    struct a_stat st;
    char buf[8];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT | O_EXCL, 0644);
    expect("open_ok", fd >= 0, 1);
    expect("size_page", syscall2(SYS_ftruncate, fd, A_PAGE), 0);
    expect("write_head", syscall4(SYS_pwrite64, fd, "head", 4, 0), 4);
    char *m = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    expect("mmap_ok", (i64)m > 0 || (i64)m < -4096, 1);
    i64 rd = a_open(D "/f", O_RDONLY, 0);
    expect("second_fd", rd >= 0, 1);
    expect("unlink_mapped", u(D "/f"), 0);
    expect("close_map_fd", a_close(fd), 0);
    expect("map_reads_head", mem_eq(m, "head", 4), 1);
    memcpy(m + 100, "tail", 4);
    expect("pread_sees_map_write", syscall4(SYS_pread64, rd, buf, 4, 100), 4);
    expect("map_write_seen", mem_eq(buf, "tail", 4), 1);
    expect("fstat_rd", syscall2(SYS_fstat, rd, &st), 0);
    expect("nlink_0", st.st_nlink, 0);
    expect("size_page_kept", st.st_size, A_PAGE);
    expect("munmap", syscall2(SYS_munmap, m, A_PAGE), 0);
    expect("close_rd", a_close(rd), 0);
    expect("cleanup_dir", syscall1(SYS_rmdir, D), 0);
    return finish();
}
