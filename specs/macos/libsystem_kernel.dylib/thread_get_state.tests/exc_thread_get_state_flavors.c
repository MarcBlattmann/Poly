/* poly-mac: libsystem
 * thread_get_state(thread, flavor, state, &count) on a thread of the program (here its own thread): the flavors a 64-bit program uses succeed with
 * a count of 32-bit words. arm64: ARM_THREAD_STATE64 (6) 68, ARM_EXCEPTION_STATE64 (7) 4, ARM_NEON_STATE64 (17) 132, ARM_DEBUG_STATE64 (15) 130,
 * ARM_THREAD_STATE (1) 70, ARM_VFP_STATE (2) 65. x86_64: x86_THREAD_STATE64 (4) 42, x86_EXCEPTION_STATE64 (6) 4, x86_FLOAT_STATE64 (5) 131,
 * x86_AVX_STATE64 (17) 211, x86_DEBUG_STATE64 (11) 16, x86_THREAD_STATE (7) 44, x86_FLOAT_STATE (8) 133. The count is in and out. */
#include "poly_mexc.h"
static unsigned count_of(mach_port_t th, int flavor, kern_return_t *kr) {
    natural_t s[1300];
    mach_msg_type_number_t n = 1300;
    *kr = thread_get_state(th, flavor, (thread_state_t)s, &n);
    return *kr ? 0 : n;
}
int test_main(void) {
    mach_port_t me = mach_thread_self();
    kern_return_t kr;
    struct {
        int flavor;
        unsigned count;
        const char *name;
    } fl[] = {
#if defined(__arm64__)
        {ARM_THREAD_STATE64, 68, "ARM_THREAD_STATE64"}, {ARM_EXCEPTION_STATE64, 4, "ARM_EXCEPTION_STATE64"}, {ARM_NEON_STATE64, 132, "ARM_NEON_STATE64"},
        {ARM_DEBUG_STATE64, 130, "ARM_DEBUG_STATE64"}, {ARM_THREAD_STATE, 70, "ARM_THREAD_STATE"}, {ARM_VFP_STATE, 65, "ARM_VFP_STATE"},
#else
        {x86_THREAD_STATE64, 42, "x86_THREAD_STATE64"}, {x86_EXCEPTION_STATE64, 4, "x86_EXCEPTION_STATE64"}, {x86_FLOAT_STATE64, 131, "x86_FLOAT_STATE64"},
        {x86_AVX_STATE64, 211, "x86_AVX_STATE64"}, {x86_DEBUG_STATE64, 16, "x86_DEBUG_STATE64"}, {x86_THREAD_STATE, 44, "x86_THREAD_STATE"},
        {x86_FLOAT_STATE, 133, "x86_FLOAT_STATE"},
#endif
    };
    for (unsigned i = 0; i < sizeof fl / sizeof fl[0]; i++) {
        char name[96];
        snprintf(name, sizeof name, "%s: succeeds with count %u", fl[i].name, fl[i].count);
        unsigned c = count_of(me, fl[i].flavor, &kr);
        check(name, kr == KERN_SUCCESS && c == fl[i].count, kr ? kr : (long long)c);
    }
    check("the count is the number of 32-bit words of the structure", EXC_STATE_COUNT * 4 == sizeof(exc_ts_t), sizeof(exc_ts_t));
    natural_t s[1300];
    mach_msg_type_number_t n = 1300;
    kr = thread_get_state(me, EXC_FLAVOR, (thread_state_t)s, &n);
    exc_ts_t *st = (exc_ts_t *)s;
    check("the state of the calling thread: a pc that is not 0", kr == KERN_SUCCESS && exc_get_pc(st) != 0, kr);
    return finish();
}
