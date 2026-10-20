/* read: a 40000-byte file read in one call and in odd-sized pieces, into
 * memory that spans pages; a buffer partly unmapped gets a short read. */
#include "a_sys.h"

#define F "/tmp/a_read_large"
#define SIZE 40000

static unsigned char data[SIZE], back[SIZE + 16];

int test_main(void) {
    for (int i = 0; i < SIZE; i++) data[i] = (unsigned char)(i * 7 + i / 251);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("create", fd, 3);
    expect("write_all", a_write(fd, data, SIZE), SIZE);
    expect("rewind", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("read_all_at_once", a_read(fd, back, sizeof back), SIZE);
    check("contents_match", mem_eq(data, back, SIZE), 0);

    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    memset(back, 0, sizeof back);
    i64 total = 0, n;
    int calls = 0;
    while ((n = a_read(fd, back + total, 997)) > 0) total += n, calls++;
    expect("pieces_total", total, SIZE);
    expect("pieces_calls", calls, 41);
    expect("pieces_end_is_0", n, 0);
    check("pieces_contents_match", mem_eq(data, back, SIZE), 0);

    /* Two pages mapped, the second then unmapped: a read that runs into it
     * stops at the end of the first page. */
    i64 m = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("munmap_second_page", syscall2(SYS_munmap, m + A_PAGE, A_PAGE), 0);
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    n = a_read(fd, (void *)(m + A_PAGE - 100), 300);
    expect("read_into_partly_mapped_is_short", n, 100);
    check("short_read_contents", mem_eq((void *)(m + A_PAGE - 100), data, 100), 0);
    expect("offset_after_short_read", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 100);
    syscall2(SYS_munmap, m, A_PAGE);

    expect("close", a_close(fd), 0);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
