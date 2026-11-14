/* proc_info, BSD 336, errors: ESRCH for a pid that does not exist, EINVAL
 * for a call number or flavor that does not exist, EFAULT for an unwritable
 * buffer. */
#include "poly_mtest.h"

int test_main(void) {
    u8 buf[256];
    i64 me = bsd0(SYS_getpid).ret;
    check_err("a pid that does not exist: ESRCH", bsd6(SYS_proc_info, PROC_INFO_CALL_PIDINFO, 99999999, PROC_PIDTBSDINFO, 0, buf, 136), ESRCH);
    check_err("flavor 9999: EINVAL", bsd6(SYS_proc_info, PROC_INFO_CALL_PIDINFO, me, 9999, 0, buf, 136), EINVAL);
    check_err("call 99: EINVAL", bsd6(SYS_proc_info, 99, me, PROC_PIDTBSDINFO, 0, buf, 136), EINVAL);
    check_err("the buffer at address 8: EFAULT", bsd6(SYS_proc_info, PROC_INFO_CALL_PIDINFO, me, PROC_PIDTBSDINFO, 0, (void *)8, 136), EFAULT);
    check_ok("a good call afterwards", bsd6(SYS_proc_info, PROC_INFO_CALL_PIDINFO, me, PROC_PIDTBSDINFO, 0, buf, 136));
    return finish();
}
