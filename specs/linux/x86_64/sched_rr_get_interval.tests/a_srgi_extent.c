/* sched_rr_get_interval writes exactly 16 bytes (tv_sec, tv_nsec), also
 * at an unaligned address: the bytes around the buffer stay as they were. */
#include "a_sys.h"

#define SYS_sched_rr_get_interval 148

int test_main(void) {
    u8 buf[48];
    for (int i = 0; i < 48; i++) buf[i] = 0xa5;
    expect("unaligned_call", syscall2(SYS_sched_rr_get_interval, 0, buf + 3), 0);
    int before_ok = 1, after_ok = 1;
    for (int i = 0; i < 3; i++) before_ok &= buf[i] == 0xa5;
    for (int i = 19; i < 48; i++) after_ok &= buf[i] == 0xa5;
    check("before_untouched", before_ok, buf[2]);
    check("after_untouched", after_ok, buf[19]);
    i64 sec = 0, nsec = 0;
    for (int i = 7; i >= 0; i--) sec = (sec << 8) | buf[3 + i];
    for (int i = 7; i >= 0; i--) nsec = (nsec << 8) | buf[11 + i];
    check("sec_written", sec == 0, sec);
    check("nsec_written", nsec >= 0 && nsec < 1000000000, nsec);
    return finish();
}
