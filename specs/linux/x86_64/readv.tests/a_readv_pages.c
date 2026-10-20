/* readv: many buffers spread over several pages read a file of 20000
 * bytes exactly as one read would. */
#include "a_sys.h"

#define F "/tmp/a_readv_pages"
#define SIZE 20000

static unsigned char data[SIZE], back[SIZE];
static struct a_iovec iov[64];

int test_main(void) {
    for (int i = 0; i < SIZE; i++) data[i] = (unsigned char)(i ^ (i >> 8));
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("write_data", a_write(fd, data, SIZE), SIZE);
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    /* 63 buffers of 312 bytes and a last one of 344. */
    for (int i = 0; i < 64; i++) iov[i].iov_base = back + i * 312, iov[i].iov_len = i < 63 ? 312 : SIZE - 63 * 312;
    expect("readv_64_buffers", syscall3(SYS_readv, fd, iov, 64), SIZE);
    check("contents_match", mem_eq(back, data, SIZE), 0);
    expect("then_eof", syscall3(SYS_readv, fd, iov, 64), 0);
    /* The same through descending buffer addresses. */
    memset(back, 0, SIZE);
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    for (int i = 0; i < 4; i++) iov[i].iov_base = back + (3 - i) * 5000, iov[i].iov_len = 5000;
    expect("readv_descending", syscall3(SYS_readv, fd, iov, 4), SIZE);
    check("descending_first_part", mem_eq(back + 15000, data, 5000), 0);
    check("descending_last_part", mem_eq(back, data + 15000, 5000), 0);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
