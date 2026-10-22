/* RLIMIT_FSIZE limits growing by truncate: a length above the soft limit
 * that is larger than the current size is EFBIG and the size stays;
 * growing up to the limit is allowed; shrinking, or keeping the size,
 * is allowed whatever the limit, also from above it. SIGXFSZ is
 * ignored here so the failing call returns. Results are printed only
 * after the limit is lifted again (stdout may be a file). */
#include "a_sys.h"

#define F "/tmp/a_truncate_fsize_limit"
#define SIGXFSZ 25

static i64 r[12];
static i64 sz[12];

static void set_limit(u64 cur) {
    struct a_rlimit rl;
    syscall4(SYS_prlimit64, 0, RLIMIT_FSIZE, 0, &rl);
    rl.rlim_cur = cur;
    syscall4(SYS_prlimit64, 0, RLIMIT_FSIZE, &rl, 0);
}

static i64 size(void) {
    struct a_stat st;
    a_stat_path(F, &st);
    return st.st_size;
}

int test_main(void) {
    struct a_sigaction sa = {SIG_IGN, 0, 0, 0};
    expect("ignore_sigxfsz", syscall4(SYS_rt_sigaction, SIGXFSZ, &sa, 0, 8), 0);
    expect("setup_file", a_make_file(F, "hello", 0644), 0);
    expect("setup_grow", syscall2(SYS_truncate, F, 10000), 0);
    set_limit(4096);
    r[0] = syscall2(SYS_truncate, F, 10000); /* same size, above the limit */
    sz[0] = size();
    r[1] = syscall2(SYS_truncate, F, 6000); /* shrink, still above */
    sz[1] = size();
    r[2] = syscall2(SYS_truncate, F, 6001); /* grow by one, above */
    sz[2] = size();
    r[3] = syscall2(SYS_truncate, F, 100);
    r[4] = syscall2(SYS_truncate, F, 4096); /* grow to the limit */
    sz[4] = size();
    r[5] = syscall2(SYS_truncate, F, 4097);
    sz[5] = size();
    r[6] = syscall2(SYS_truncate, F, 1L << 40);
    set_limit(0);
    r[7] = syscall2(SYS_truncate, F, 1);
    r[8] = syscall2(SYS_truncate, F, 0);
    r[9] = syscall2(SYS_truncate, F, 0);
    r[10] = syscall2(SYS_truncate, F, 1);
    sz[10] = size();
    set_limit(RLIM_INFINITY);
    expect("same_size_above_limit", r[0], 0);
    expect("same_size_size", sz[0], 10000);
    expect("shrink_above_limit", r[1], 0);
    expect("shrink_size", sz[1], 6000);
    expect("grow_above_limit", r[2], -EFBIG);
    expect("grow_above_size_kept", sz[2], 6000);
    expect("shrink_below", r[3], 0);
    expect("grow_to_limit", r[4], 0);
    expect("grow_to_limit_size", sz[4], 4096);
    expect("grow_past_limit", r[5], -EFBIG);
    expect("grow_past_size_kept", sz[5], 4096);
    expect("grow_huge", r[6], -EFBIG);
    expect("limit0_shrink", r[7], 0);
    expect("limit0_to_zero", r[8], 0);
    expect("limit0_zero_again", r[9], 0);
    expect("limit0_grow", r[10], -EFBIG);
    expect("limit0_size", sz[10], 0);
    expect("unlimited_grow", syscall2(SYS_truncate, F, 8192), 0);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
