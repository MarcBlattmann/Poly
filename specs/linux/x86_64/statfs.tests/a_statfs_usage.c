/* statfs: writing 1 MiB to a file does not make more blocks free (f_bfree,
 * f_bavail), and removing it does not make fewer; the totals, the block
 * size and the type stay the same.
 *
 * Measured in a fresh directory on the tmpfs at /dev/shm (on Linux and on
 * Poly), not on /tmp: the free counts are those of the whole file system,
 * and those of /tmp move with whatever else the machine writes. Another
 * process could still write or free space on /dev/shm between two
 * measurements, so each measurement is as short as it can be (one write
 * of 1 MiB between the first two statfs calls, close and unlink between
 * the last two), and the test makes ROUNDS rounds with the same calls
 * every time, at growing distances (5, 10, 15 ... ms), and passes when one
 * round shows all of it: a wrong answer shows in every round, an outside
 * change only in the rounds it falls into. The checks printed are those of
 * the first round that shows all of it, else of the last round. */
#include "a_sys.h"

#define TMPFS_MAGIC 0x01021994 /* linux/magic.h */
#define D "/dev/shm/a_statfs_usage"
#define F D "/f"
#define ROUNDS 12
#define SIZE (1 << 20)

static char data[SIZE];

struct round {
    int calls_ok;       /* every statfs, open, write, close and unlink succeeded */
    int totals_same;    /* f_blocks, f_bsize, f_frsize unchanged */
    int free_not_more;  /* while the file holds 1 MiB */
    int avail_not_more;
    int free_back;      /* after unlink: not fewer than while it was there */
    int avail_back;
    int type_same;
    i64 free_rise;      /* printed when free_not_more fails */
};

static int all(const struct round *r) {
    return r->calls_ok && r->totals_same && r->free_not_more && r->avail_not_more && r->free_back && r->avail_back &&
           r->type_same;
}

static void one_round(struct round *r) {
    struct a_statfs before, during, after;
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    int ok = fd >= 0;
    ok &= syscall2(SYS_statfs, D, &before) == 0;
    ok &= a_write(fd, data, SIZE) == SIZE;
    ok &= syscall2(SYS_statfs, D, &during) == 0;
    ok &= a_close(fd) == 0;
    ok &= a_unlink(F) == 0;
    ok &= syscall2(SYS_statfs, D, &after) == 0;
    r->calls_ok = ok;
    r->totals_same = during.f_blocks == before.f_blocks && after.f_blocks == before.f_blocks &&
                     during.f_bsize == before.f_bsize && after.f_bsize == before.f_bsize &&
                     during.f_frsize == before.f_frsize && after.f_frsize == before.f_frsize;
    r->free_not_more = during.f_bfree <= before.f_bfree;
    r->avail_not_more = during.f_bavail <= before.f_bavail;
    r->free_back = after.f_bfree >= during.f_bfree;
    r->avail_back = after.f_bavail >= during.f_bavail;
    r->type_same = during.f_type == before.f_type && after.f_type == before.f_type;
    r->free_rise = (i64)(during.f_bfree - before.f_bfree);
}

int test_main(void) {
    struct a_statfs shm;
    i64 r = syscall2(SYS_statfs, "/dev/shm", &shm);
    check("setup_dev_shm_is_tmpfs", r == 0 && shm.f_type == TMPFS_MAGIC, r == 0 ? shm.f_type : r);
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    memset(data, 'u', sizeof data);
    struct round rounds[ROUNDS], *shown = &rounds[ROUNDS - 1];
    for (int i = 0; i < ROUNDS; i++) {
        if (i > 0) {
            struct a_timespec gap = {0, 5000000L * i};
            syscall2(SYS_nanosleep, &gap, 0);
        }
        one_round(&rounds[i]);
    }
    for (int i = ROUNDS - 1; i >= 0; i--)
        if (all(&rounds[i])) shown = &rounds[i];
    check("calls_ok", shown->calls_ok, 0);
    check("totals_unchanged", shown->totals_same, 0);
    check("free_not_more_while_written", shown->free_not_more, shown->free_rise);
    check("avail_not_more_while_written", shown->avail_not_more, 0);
    check("free_not_less_after_unlink", shown->free_back, 0);
    check("avail_not_less_after_unlink", shown->avail_back, 0);
    check("type_stable", shown->type_same, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
