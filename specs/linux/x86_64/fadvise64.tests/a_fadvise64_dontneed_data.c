/* fadvise64 DONTNEED never loses data: clean data, data just written
 * (dirty) and data written through a shared mapping all read back the same
 * afterwards, through read and through the mapping. */
#include "a_sys.h"

#define SYS_fadvise64 221
#define D "/tmp/fadvise64_dontneed_data"

static i64 fadv(i64 fd, i64 off, i64 len, i64 advice) { return syscall4(SYS_fadvise64, fd, off, len, advice); }

static int all_are(const char *p, size_t n, char c) {
    for (size_t i = 0; i < n; i++)
        if (p[i] != c) return 0;
    return 1;
}

int test_main(void) {
    static char buf[4 * 4096], back[4 * 4096];
    memset(buf, 'c', sizeof buf);
    a_mkdir(D, 0755);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", fd >= 0, fd);
    expect("write", a_write(fd, buf, sizeof buf), sizeof buf);
    expect("fsync", syscall1(SYS_fsync, fd), 0);
    expect("dontneed_clean", fadv(fd, 0, 0, 4), 0);
    expect("pread_clean", syscall4(SYS_pread64, fd, back, sizeof back, 0), sizeof back);
    check("clean_data_kept", all_are(back, sizeof back, 'c'), back[0]);
    expect("pwrite_dirty", syscall4(SYS_pwrite64, fd, "DIRTY", 5, 4096), 5);
    expect("dontneed_dirty", fadv(fd, 0, 0, 4), 0);
    expect("pread_dirty", syscall4(SYS_pread64, fd, back, 5, 4096), 5);
    check("dirty_data_kept", mem_eq(back, "DIRTY", 5), back[0]);
    i64 m = syscall6(SYS_mmap, 0, sizeof buf, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("mmap", m > 0 || m < -4096, m);
    volatile char *p = (volatile char *)m;
    p[2 * 4096] = 'M';
    p[2 * 4096 + 1] = 'A';
    expect("dontneed_mapped", fadv(fd, 0, sizeof buf, 4), 0);
    expect("pread_mapped", syscall4(SYS_pread64, fd, back, 2, 2 * 4096), 2);
    check("mapped_write_seen_by_read", back[0] == 'M' && back[1] == 'A', back[0]);
    check("mapping_kept", p[2 * 4096] == 'M' && p[4096] == 'D' && p[3 * 4096] == 'c', p[2 * 4096]);
    expect("dontneed_partial", fadv(fd, 10, 4000, 4), 0);
    check("mapping_kept_partial", p[10] == 'c' && p[2 * 4096 + 1] == 'A', p[10]);
    syscall2(SYS_munmap, m, sizeof buf);
    expect("dontneed_after_unmap", fadv(fd, 0, 0, 4), 0);
    expect("pread_final", syscall4(SYS_pread64, fd, back, sizeof back, 0), sizeof back);
    check("final_page0", all_are(back, 4096, 'c'), back[0]);
    check("final_page1", mem_eq(back + 4096, "DIRTY", 5) && all_are(back + 4096 + 5, 4096 - 5, 'c'), back[4096]);
    check("final_page2", back[2 * 4096] == 'M' && back[2 * 4096 + 1] == 'A' && all_are(back + 2 * 4096 + 2, 4094, 'c'),
          back[2 * 4096]);
    check("final_page3", all_are(back + 3 * 4096, 4096, 'c'), back[3 * 4096]);
    a_close(fd);
    a_unlink(D "/f");
    a_rmdir(D);
    return finish();
}
