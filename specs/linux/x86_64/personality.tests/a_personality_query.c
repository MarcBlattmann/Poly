/* personality: a process starts with its parent's persona: PER_LINUX
 * (0), possibly with ADDR_NO_RANDOMIZE (0x0040000), which the test
 * runner sets. Setting a persona returns the previous one; 0xffffffff
 * only queries (returns the current persona and changes nothing). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_personality 135
/* linux/personality.h */
#define PER_LINUX 0x0000
#define PER_BSD 0x0006
#define PER_LINUX32 0x0008
#define PER_SVR4 0x04100001
#define ADDR_NO_RANDOMIZE 0x0040000
#define QUERY 0xffffffffUL

static i64 personality(u64 p) { return syscall1(SYS_personality, p); }

int test_main(void) {
    i64 initial = personality(QUERY);
    expect("initial_linux", initial & ~(i64)ADDR_NO_RANDOMIZE, PER_LINUX);
    expect("query_again_same", personality(QUERY), initial);
    expect("set_linux32_returns_old", personality(PER_LINUX32), initial);
    expect("query_linux32", personality(QUERY), PER_LINUX32);
    expect("query_does_not_change", personality(QUERY), PER_LINUX32);
    expect("set_bsd_returns_linux32", personality(PER_BSD), PER_LINUX32);
    expect("set_same_returns_same", personality(PER_BSD), PER_BSD);
    expect("set_svr4_returns_bsd", personality(PER_SVR4), PER_BSD);
    expect("query_svr4", personality(QUERY), PER_SVR4);
    expect("set_flag_only", personality(ADDR_NO_RANDOMIZE), PER_SVR4);
    expect("query_flag_only", personality(QUERY), ADDR_NO_RANDOMIZE);
    expect("back_to_linux", personality(PER_LINUX), ADDR_NO_RANDOMIZE);
    expect("query_linux", personality(QUERY), PER_LINUX);
    return finish();
}
