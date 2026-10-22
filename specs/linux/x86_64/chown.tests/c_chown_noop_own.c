/* chown on a file the caller owns: -1 for an id (as a 32-bit value) leaves
 * it unchanged; the caller's own uid and gid are accepted; 0 is returned. */
#include "a_sys.h"

#define D "/tmp/c_chown_noop_own"

static i64 ch(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }

int test_main(void) {
    struct a_stat st;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("both_minus_one", ch(D "/f", -1, -1), 0);
    a_stat_path(D "/f", &st);
    check("ids_unchanged", st.st_uid == (u32)uid && st.st_gid == (u32)gid, st.st_uid);
    expect("own_ids", ch(D "/f", uid, gid), 0);
    a_stat_path(D "/f", &st);
    check("own_ids_kept", st.st_uid == (u32)uid && st.st_gid == (u32)gid, st.st_uid);
    expect("own_uid_only", ch(D "/f", uid, -1), 0);
    expect("own_gid_only", ch(D "/f", -1, gid), 0);
    expect("ffffffff_both", ch(D "/f", 0xffffffffL, 0xffffffffL), 0);
    a_stat_path(D "/f", &st);
    check("ffffffff_unchanged", st.st_uid == (u32)uid && st.st_gid == (u32)gid, st.st_uid);
    check("size_kept", st.st_size == 4, st.st_size);
    check("mode_kept", (st.st_mode & 07777) == 0644, st.st_mode & 07777);
    check("type_kept", (st.st_mode & S_IFMT) == S_IFREG, st.st_mode & S_IFMT);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
