/* statfs: writing 1 MiB to a file in /tmp does not make more blocks free,
 * and removing it does not make fewer; the totals stay the same. */
#include "a_sys.h"

#define F "/tmp/a_statfs_usage"

static char block[65536];

int test_main(void) {
    struct a_statfs before, during, after;
    expect("statfs_before", syscall2(SYS_statfs, "/tmp", &before), 0);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    memset(block, 'u', sizeof block);
    int ok = 1;
    for (int i = 0; i < 16; i++) ok &= a_write(fd, block, sizeof block) == sizeof block;
    check("write_1mib", ok, 0);
    expect("statfs_during", syscall2(SYS_statfs, "/tmp", &during), 0);
    check("totals_unchanged", during.f_blocks == before.f_blocks && during.f_bsize == before.f_bsize, 0);
    check("free_not_more", during.f_bfree <= before.f_bfree, 0);
    a_close(fd);
    expect("unlink", a_unlink(F), 0);
    expect("statfs_after", syscall2(SYS_statfs, "/tmp", &after), 0);
    check("free_not_less_after_unlink", after.f_bfree >= during.f_bfree, 0);
    check("type_stable", after.f_type == before.f_type, 0);
    return finish();
}
