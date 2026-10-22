/* A successful fchown updates the change time (also with -1 for both
 * ids, and through a read-only descriptor); the modification and access
 * times stay. A failed call (EPERM) changes no time. */
#include "a_sys.h"

#define F "/tmp/c_fchown_times"

struct ts {
    i64 sec, nsec;
};

static void stamps(i64 fd, struct ts *c, struct ts *m, struct ts *a) {
    struct a_statx sx;
    syscall5(SYS_statx, fd, "", AT_EMPTY_PATH, 0x7ff, &sx);
    c->sec = sx.stx_ctime.tv_sec;
    c->nsec = sx.stx_ctime.tv_nsec;
    m->sec = sx.stx_mtime.tv_sec;
    m->nsec = sx.stx_mtime.tv_nsec;
    a->sec = sx.stx_atime.tv_sec;
    a->nsec = sx.stx_atime.tv_nsec;
}

static int same(struct ts x, struct ts y) { return x.sec == y.sec && x.nsec == y.nsec; }
static int later(struct ts x, struct ts y) { return x.sec > y.sec || (x.sec == y.sec && x.nsec > y.nsec); }

static void pause_20ms(void) {
    struct ts t = {0, 20000000};
    syscall2(SYS_nanosleep, &t, 0);
}

int test_main(void) {
    struct ts c0, m0, a0, c1, m1, a1;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    a_unlink(F);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_EXCL, 0644);
    check("setup_open", fd >= 0, fd);
    stamps(fd, &c0, &m0, &a0);
    pause_20ms();
    expect("minus_one", syscall3(SYS_fchown, fd, -1, -1), 0);
    stamps(fd, &c1, &m1, &a1);
    check("minus_one_ctime_later", later(c1, c0), c1.sec);
    check("minus_one_mtime_same", same(m1, m0), m1.sec);
    check("minus_one_atime_same", same(a1, a0), a1.sec);
    c0 = c1;
    pause_20ms();
    expect("own_ids", syscall3(SYS_fchown, fd, uid, gid), 0);
    stamps(fd, &c1, &m1, &a1);
    check("own_ids_ctime_later", later(c1, c0), c1.sec);
    check("own_ids_mtime_same", same(m1, m0), m1.sec);
    check("own_ids_atime_same", same(a1, a0), a1.sec);
    c0 = c1;
    i64 rd = a_open(F, O_RDONLY, 0);
    check("open_rdonly", rd >= 0, rd);
    pause_20ms();
    expect("rdonly", syscall3(SYS_fchown, rd, -1, -1), 0);
    stamps(fd, &c1, &m1, &a1);
    check("rdonly_ctime_later", later(c1, c0), c1.sec);
    check("rdonly_mtime_same", same(m1, m0), m1.sec);
    a_close(rd);
    if (uid != 0) {
        c0 = c1;
        pause_20ms();
        expect("foreign_uid_eperm", syscall3(SYS_fchown, fd, uid + 1, -1), -EPERM);
        stamps(fd, &c1, &m1, &a1);
        check("eperm_ctime_same", same(c1, c0), c1.sec);
    }
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
