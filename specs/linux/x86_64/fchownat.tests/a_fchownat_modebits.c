/* fchownat: changing the owner of a regular file clears its set-user-ID
 * bit, and its set-group-ID bit when group execute is set; a directory's
 * bits are kept. Even a chown with -1 for both ids clears them. */
#include "a_sys.h"

#define D "/tmp/a_fchownat_modebits"

static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)(st.st_mode & 07777);
}

static i64 chown3(const char *p, i64 uid, i64 gid) { return syscall5(SYS_fchownat, AT_FDCWD, p, uid, gid, 0); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 06755);
    expect("bits_set", mode_of(D "/f"), 06755);
    expect("noop_chown", chown3(D "/f", -1, -1), 0);
    expect("noop_clears_too", mode_of(D "/f"), 0755);
    syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 06755);
    expect("chown_uid", chown3(D "/f", 1000, -1), 0);
    expect("suid_and_sgid_cleared", mode_of(D "/f"), 0755);
    syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 06745);
    expect("chown_gid", chown3(D "/f", -1, 1000), 0);
    expect("sgid_without_group_exec_kept", mode_of(D "/f"), 02745);
    syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 01755);
    expect("chown_with_sticky", chown3(D "/f", 0, 0), 0);
    expect("sticky_kept", mode_of(D "/f"), 01755);
    expect("setup_dir", a_mkdir(D "/d", 0700), 0);
    syscall3(SYS_fchmodat, AT_FDCWD, D "/d", 07755);
    expect("chown_dir", chown3(D "/d", 1000, 1000), 0);
    expect("dir_bits_kept", mode_of(D "/d"), 07755);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
