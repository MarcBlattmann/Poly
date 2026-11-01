/* truncate(2) needs write permission on the file (EACCES; root passes)
 * and search permission on the way; ownership does not matter. Above
 * RLIMIT_FSIZE it fails with EFBIG and sends SIGXFSZ (ignored here). */
#include "a_sys.h"

#define D "/tmp/lb_truncate_perm"
#define SIGXFSZ 25

static i64 truncate_(const char *p, i64 len) { return syscall2(SYS_truncate, p, len); }
static i64 size_of(const char *p) {
    struct a_stat st;
    if (a_stat_path(p, &st)) return -1;
    return st.st_size;
}

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "abcdef", 0644), 0);
    expect("chmod_0444", syscall2(SYS_chmod, D "/f", 0444), 0);
    expect("readonly_file", truncate_(D "/f", 1), root ? 0 : -EACCES);
    expect("size_after_ro", size_of(D "/f"), root ? 1 : 6);
    expect("chmod_0200", syscall2(SYS_chmod, D "/f", 0200), 0);
    expect("writeonly_file", truncate_(D "/f", 2), 0);
    expect("chmod_0", syscall2(SYS_chmod, D "/f", 0), 0);
    expect("mode0_file", truncate_(D "/f", 3), root ? 0 : -EACCES);
    /* order: a missing file in a writable directory is ENOENT; EISDIR on a
     * directory without write permission */
    expect("setup_rodir", a_mkdir(D "/rodir", 0555), 0);
    expect("rodir_eisdir", truncate_(D "/rodir", 0), -EISDIR);
    expect("chmod_f_0644", syscall2(SYS_chmod, D "/f", 0644), 0);
    /* search permission */
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_g", a_make_file(D "/sub/g", "x", 0666), 0);
    expect("chmod_sub_0600", syscall2(SYS_chmod, D "/sub", 0600), 0);
    expect("closed_dir", truncate_(D "/sub/g", 0), root ? 0 : -EACCES);
    expect("chmod_sub_back", syscall2(SYS_chmod, D "/sub", 0755), 0);

    /* RLIMIT_FSIZE */
    struct a_sigaction ign = {SIG_IGN, 0, 0, 0};
    expect("ignore_sigxfsz", syscall4(SYS_rt_sigaction, SIGXFSZ, &ign, 0, 8), 0);
    /* the limit applies to the test's output too: collect, lift, check */
    struct a_rlimit lim = {1000, RLIM_INFINITY}, unlimited = {RLIM_INFINITY, RLIM_INFINITY};
    i64 set = syscall2(SYS_setrlimit, RLIMIT_FSIZE, &lim);
    i64 to_limit = truncate_(D "/f", 1000);
    i64 past = truncate_(D "/f", 1001);
    i64 size_at = size_of(D "/f");
    i64 shrink = truncate_(D "/f", 10);
    i64 unset = syscall2(SYS_setrlimit, RLIMIT_FSIZE, &unlimited);
    expect("setrlimit", set, 0);
    expect("grow_to_limit", to_limit, 0);
    expect("grow_past_limit", past, -EFBIG);
    expect("size_at_limit", size_at, 1000);
    expect("shrink_under_limit", shrink, 0);
    expect("unset_rlimit", unset, 0);
    u64 pending = 0;
    expect("sigpending", syscall2(SYS_rt_sigpending, &pending, 8), 0);
    expect("ignored_not_pending", pending & (1UL << (SIGXFSZ - 1)), 0);

    expect("cleanup_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rodir", a_rmdir(D "/rodir"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
