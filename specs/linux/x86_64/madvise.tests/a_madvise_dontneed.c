/* madvise MADV_DONTNEED: private anonymous pages read as zero afterwards;
 * a private file mapping reads the file again; a shared anonymous mapping
 * keeps its data. */
#include "a_sys.h"

#define F "/tmp/a_madvise_dontneed"

int test_main(void) {
    i64 m = syscall6(SYS_mmap, 0, 3 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    char *p = (char *)m;
    memset(p, 'd', 3 * A_PAGE);
    expect("dontneed_middle", syscall3(SYS_madvise, m + A_PAGE, A_PAGE, MADV_DONTNEED), 0);
    check("first_page_kept", p[0] == 'd' && p[A_PAGE - 1] == 'd', p[0]);
    check("middle_zeroed", p[A_PAGE] == 0 && p[2 * A_PAGE - 1] == 0, p[A_PAGE]);
    check("last_page_kept", p[2 * A_PAGE] == 'd', p[2 * A_PAGE]);
    p[A_PAGE] = 'n';
    check("zeroed_page_writable", p[A_PAGE] == 'n', p[A_PAGE]);
    syscall2(SYS_munmap, m, 3 * A_PAGE);

    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    static char page[A_PAGE];
    memset(page, 'F', sizeof page);
    a_write(fd, page, sizeof page);
    m = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
    p = (char *)m;
    p[0] = 'c';
    check("private_copy_changed", p[0] == 'c', p[0]);
    expect("dontneed_file_private", syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED), 0);
    check("file_contents_back", p[0] == 'F', p[0]);
    syscall2(SYS_munmap, m, A_PAGE);
    a_close(fd);
    a_unlink(F);

    m = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    p = (char *)m;
    p[10] = 's';
    expect("dontneed_shared", syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED), 0);
    check("shared_data_kept", p[10] == 's', p[10]);
    syscall2(SYS_munmap, m, A_PAGE);
    return finish();
}
