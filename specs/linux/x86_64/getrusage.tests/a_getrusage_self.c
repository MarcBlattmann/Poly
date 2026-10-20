/* getrusage RUSAGE_SELF: times are well-formed and grow with CPU work;
 * maxrss is positive; touching new pages counts minor faults. */
#include "a_sys.h"

static i64 usec(struct a_timeval *t) { return t->tv_sec * 1000000 + t->tv_usec; }

int test_main(void) {
    struct a_rusage a, b;
    expect("self", syscall2(SYS_getrusage, RUSAGE_SELF, &a), 0);
    check("well_formed", a.ru_utime.tv_usec >= 0 && a.ru_utime.tv_usec < 1000000 && a.ru_stime.tv_usec >= 0 &&
                             a.ru_stime.tv_usec < 1000000 && a.ru_utime.tv_sec >= 0, a.ru_utime.tv_usec);
    check("maxrss_positive", a.ru_maxrss > 0, a.ru_maxrss);
    /* About 0.3 s of CPU (1e9 TSC ticks at ~3 GHz). */
    a_spin_ticks(1000000000UL);
    syscall2(SYS_getrusage, RUSAGE_SELF, &b);
    check("cpu_time_grew", usec(&b.ru_utime) + usec(&b.ru_stime) > usec(&a.ru_utime) + usec(&a.ru_stime),
          usec(&b.ru_utime));
    i64 m = syscall6(SYS_mmap, 0, 64 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_getrusage, RUSAGE_SELF, &a);
    for (int i = 0; i < 64; i++) ((volatile char *)m)[i * A_PAGE] = 1;
    syscall2(SYS_getrusage, RUSAGE_SELF, &b);
    check("minor_faults_counted", b.ru_minflt - a.ru_minflt >= 32, b.ru_minflt - a.ru_minflt);
    check("maxrss_not_less", b.ru_maxrss >= a.ru_maxrss, b.ru_maxrss);
    syscall2(SYS_munmap, m, 64 * A_PAGE);
    return finish();
}
