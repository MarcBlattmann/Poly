/* sysinfo: memory sizes in mem_unit bytes, free not above total, at least
 * one process, uptime in seconds. */
#include "a_sys.h"

int test_main(void) {
    struct a_sysinfo s;
    memset(&s, 0xff, sizeof s);
    expect("sysinfo", syscall1(SYS_sysinfo, &s), 0);
    check("uptime_nonnegative", s.uptime >= 0, s.uptime);
    check("totalram_positive", s.totalram > 0, (i64)s.totalram);
    check("freeram_le_total", s.freeram <= s.totalram, (i64)s.freeram);
    check("freeswap_le_total", s.freeswap <= s.totalswap, (i64)s.freeswap);
    check("mem_unit_at_least_1", s.mem_unit >= 1, s.mem_unit);
    check("at_least_one_process", s.procs >= 1, s.procs);
    check("at_least_16_mib", s.totalram * s.mem_unit >= 16UL << 20, (i64)(s.totalram * s.mem_unit));
    return finish();
}
