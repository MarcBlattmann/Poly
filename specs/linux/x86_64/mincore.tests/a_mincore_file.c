/* mincore on a shared file mapping: pages of a file just written are in
 * the page cache, so they are resident even before they are touched
 * through the mapping. */
#include "a_sys.h"

#define F "/tmp/a_mincore_file"

static char data[3 * A_PAGE];

int test_main(void) {
    unsigned char vec[4];
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    memset(data, 'f', sizeof data);
    expect("write_file", a_write(fd, data, sizeof data), sizeof data);
    i64 m = syscall6(SYS_mmap, 0, 3 * A_PAGE, PROT_READ, MAP_SHARED, fd, 0);
    check("map_file", m > 0, m);
    if (m <= 0) return finish();
    expect("mincore_file", syscall3(SYS_mincore, m, 3 * A_PAGE, vec), 0);
    check("cached_pages_resident", (vec[0] & 1) && (vec[1] & 1) && (vec[2] & 1), vec[0]);
    check("map_reads_file", ((char *)m)[A_PAGE] == 'f', ((char *)m)[A_PAGE]);
    syscall2(SYS_munmap, m, 3 * A_PAGE);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
