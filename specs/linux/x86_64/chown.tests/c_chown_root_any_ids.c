/* Root may give a file any owner and group up to 2^32-2; -1 leaves an id
 * unchanged. Root's chown clears set-user-ID like anyone's, also on a file
 * it does not own, and root passes search checks on the way.
 * poly-diff: needs=root (only root may give a file away)
 */
#include "a_sys.h"

#define D "/tmp/c_chown_root_any_ids"

static i64 ch(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }
static void ids(const char *name, const char *p, u32 uid, u32 gid) {
    struct a_stat st;
    a_lstat_path(p, &st);
    check(name, st.st_uid == uid && st.st_gid == gid, st.st_uid);
}
static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)(st.st_mode & 07777);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("give_away", ch(D "/f", 1234, 5678), 0);
    ids("given", D "/f", 1234, 5678);
    expect("uid_only", ch(D "/f", 4321, -1), 0);
    ids("uid_changed_gid_kept", D "/f", 4321, 5678);
    expect("gid_only", ch(D "/f", -1, 8765), 0);
    ids("gid_changed_uid_kept", D "/f", 4321, 8765);
    expect("max_ids", ch(D "/f", 0xfffffffeL, 0xfffffffeL), 0);
    ids("max_ids_set", D "/f", 0xfffffffeU, 0xfffffffeU);
    expect("noop", ch(D "/f", -1, -1), 0);
    ids("noop_kept", D "/f", 0xfffffffeU, 0xfffffffeU);
    syscall2(SYS_chmod, D "/f", 06755);
    expect("not_owner_noop", ch(D "/f", -1, -1), 0);
    expect("not_owner_bits_cleared", mode_of(D "/f"), 0755);
    syscall2(SYS_chmod, D "/f", 04755);
    expect("back_to_root", ch(D "/f", 0, 0), 0);
    ids("root_again", D "/f", 0, 0);
    expect("root_bits_cleared", mode_of(D "/f"), 0755);
    syscall2(SYS_chmod, D, 0);
    expect("through_mode_zero_dir", ch(D "/f", 11, 12), 0);
    syscall2(SYS_chmod, D, 0700);
    ids("through_mode_zero_dir_set", D "/f", 11, 12);
    expect("dir_give_away", ch(D, 99, 98), 0);
    ids("dir_given", D, 99, 98);
    expect("dir_back", ch(D, 0, 0), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
