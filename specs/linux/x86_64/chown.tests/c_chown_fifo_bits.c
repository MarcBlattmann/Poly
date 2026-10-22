/* chown on a FIFO (not a regular file, not a directory) succeeds and
 * clears the set-user-ID bit, and the set-group-ID bit only with group
 * execute, as for a regular file. */
#include "a_sys.h"

#define D "/tmp/c_chown_fifo_bits"
/* asm/unistd_64.h */
#define SYS_mknodat 259

static i64 ch(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }
static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)(st.st_mode & 07777);
}

int test_main(void) {
    struct a_stat st;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", S_IFIFO | 0644, 0), 0);
    expect("fifo_own_ids", ch(D "/p", uid, gid), 0);
    a_lstat_path(D "/p", &st);
    check("still_fifo", (st.st_mode & S_IFMT) == S_IFIFO, st.st_mode & S_IFMT);
    syscall2(SYS_chmod, D "/p", 06644);
    expect("fifo_bits_set", mode_of(D "/p"), 06644);
    expect("fifo_noop", ch(D "/p", -1, -1), 0);
    expect("fifo_suid_cleared_sgid_kept", mode_of(D "/p"), 02644);
    syscall2(SYS_chmod, D "/p", 06654);
    expect("fifo_noop_gx", ch(D "/p", -1, -1), 0);
    expect("fifo_both_cleared", mode_of(D "/p"), 0654);
    expect("cleanup_p", a_unlink(D "/p"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
