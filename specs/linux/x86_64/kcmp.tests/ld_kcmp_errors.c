/* kcmp errors and their order: pid1 or pid2 naming no process (also 0)
 * is ESRCH, checked before the type; then a type outside 0..7 is EINVAL
 * (type is 32 bits: the register's upper half is ignored); for KCMP_FILE
 * a descriptor that is not open is EBADF.
 * poly-diff: syscalls=names (pids are arguments)
 */
#include "a_sys.h"

#define SYS_kcmp 312
#define KCMP_FILE 0
#define KCMP_VM 1
#define KCMP_TYPES 8
#define NO_PID 0x7fffffff

static i64 kcmp(i64 p1, i64 p2, i64 type, i64 i1, i64 i2) { return syscall5(SYS_kcmp, p1, p2, type, i1, i2); }

int test_main(void) {
    i64 me = syscall0(SYS_getpid);
    expect("ok", kcmp(me, me, KCMP_VM, 0, 0), 0);
    expect("type8_einval", kcmp(me, me, KCMP_TYPES, 0, 0), -EINVAL);
    expect("type100_einval", kcmp(me, me, 100, 0, 0), -EINVAL);
    expect("type_minus1_einval", kcmp(me, me, -1, 0, 0), -EINVAL);
    expect("type_upper_half_ignored", kcmp(me, me, (1L << 32) | KCMP_VM, 0, 0), 0);
    expect("pid1_none_esrch", kcmp(NO_PID, me, KCMP_VM, 0, 0), -ESRCH);
    expect("pid2_none_esrch", kcmp(me, NO_PID, KCMP_VM, 0, 0), -ESRCH);
    expect("pid1_zero_esrch", kcmp(0, me, KCMP_VM, 0, 0), -ESRCH);
    expect("pid2_zero_esrch", kcmp(me, 0, KCMP_VM, 0, 0), -ESRCH);
    expect("pid_minus1_esrch", kcmp(-1, me, KCMP_VM, 0, 0), -ESRCH);
    expect("esrch_before_type", kcmp(NO_PID, me, 100, 0, 0), -ESRCH);
    expect("esrch_before_ebadf", kcmp(me, NO_PID, KCMP_FILE, 50, 50), -ESRCH);
    expect("file_ebadf", kcmp(me, me, KCMP_FILE, 50, 1), -EBADF);
    expect("file_both_ebadf", kcmp(me, me, KCMP_FILE, 50, 51), -EBADF);
    expect("file_stdio_same", kcmp(me, me, KCMP_FILE, 1, 1), 0);
    return finish();
}
