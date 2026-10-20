/* capget: header {version, pid} and two data records (effective,
 * permitted, inheritable; low 32 capabilities, then high) for version 3
 * (0x20080522); pid 0 is the caller; root has CAP_SETUID, CAP_SETGID,
 * CAP_KILL and CAP_SYS_ADMIN effective and permitted, nothing inheritable;
 * an unknown version is EINVAL and the preferred version is written back
 * into the header (also with a NULL data pointer, which then returns 0);
 * version 1 (0x19980330) has one record; a pid that does not exist is
 * ESRCH; a bad header pointer is EFAULT. */
#include "poly_ipc.h"

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

int test_main(void) {
    struct cap_header h = { 0x20080522, 0 };
    struct cap_data d[2];
    memset(d, 0x55, sizeof d);
    i64 r = syscall2(SYS_capget, &h, d);
    check("capget_v3_ok", r == 0, r);
    check("cap_setuid_effective", (d[0].effective >> 7) & 1, (i64)d[0].effective);
    check("cap_setgid_effective", (d[0].effective >> 6) & 1, 0);
    check("cap_kill_effective", (d[0].effective >> 5) & 1, 0);
    check("cap_sys_admin_effective", (d[0].effective >> 21) & 1, 0);
    check("effective_equals_permitted", d[0].effective == d[0].permitted && d[1].effective == d[1].permitted, 0);
    check("nothing_inheritable", d[0].inheritable == 0 && d[1].inheritable == 0, (i64)d[0].inheritable);
    check("high_word_above_last_cap_zero", (d[1].permitted >> 9) == 0, (i64)d[1].permitted);

    h.pid = (int)getpid();
    struct cap_data e[2];
    r = syscall2(SYS_capget, &h, e);
    check("own_pid_same", r == 0 && e[0].effective == d[0].effective, r);

    h.version = 0x12345678;
    r = syscall2(SYS_capget, &h, d);
    check("bad_version_einval", r == -EINVAL, r);
    check("preferred_version_written", h.version == 0x20080522, (i64)h.version);
    h.version = 0;
    r = syscall2(SYS_capget, &h, 0);
    check("probe_with_null_data_returns_zero", r == 0, r);
    check("probe_writes_version", h.version == 0x20080522, (i64)h.version);

    h.version = 0x19980330;
    h.pid = 0;
    struct cap_data one[2];
    memset(one, 0x55, sizeof one);
    r = syscall2(SYS_capget, &h, one);
    check("version_1_ok", r == 0, r);
    check("version_1_one_record", one[0].effective == d[0].effective && one[1].effective == 0x55555555, 0);

    h.version = 0x20080522;
    h.pid = 4194304 + 11;
    r = syscall2(SYS_capget, &h, d);
    check("no_such_pid_esrch", r == -ESRCH, r);
    r = syscall2(SYS_capget, 8, d);
    check("bad_header_efault", r == -EFAULT, r);
    return finish();
}
