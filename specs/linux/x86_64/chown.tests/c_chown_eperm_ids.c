/* A caller that is not root may not give a file away or put it in a group
 * it is not a member of: EPERM, and nothing changes (mode bits, ctime).
 * Root may set any ids. */
#include "a_sys.h"

#define D "/tmp/c_chown_eperm_ids"
/* asm/unistd_64.h */
#define SYS_getgroups 115

static i64 ch(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }
static u32 groups[65536];

/* A gid that is neither the caller's egid nor one of its groups. */
static i64 foreign_gid(i64 egid) {
    i64 n = syscall2(SYS_getgroups, 65536, groups);
    for (u32 g = 54321;; g++) {
        int used = g == (u32)egid;
        for (i64 i = 0; i < n; i++) used |= groups[i] == g;
        if (!used) return g;
    }
}

int test_main(void) {
    struct a_stat a, b;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    i64 other_uid = uid == 54321 ? 54322 : 54321;
    i64 other_gid = foreign_gid(gid);
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    syscall2(SYS_chmod, D "/f", 06755);
    a_stat_path(D "/f", &a);
    if (uid != 0) {
        expect("other_uid_eperm", ch(D "/f", other_uid, -1), -EPERM);
        expect("other_gid_eperm", ch(D "/f", -1, other_gid), -EPERM);
        expect("both_other_eperm", ch(D "/f", other_uid, other_gid), -EPERM);
        expect("own_uid_other_gid_eperm", ch(D "/f", uid, other_gid), -EPERM);
        expect("other_uid_own_gid_eperm", ch(D "/f", other_uid, gid), -EPERM);
        expect("uid_fffffffe_eperm", ch(D "/f", 0xfffffffeL, -1), -EPERM);
        expect("gid_fffffffe_eperm", ch(D "/f", -1, 0xfffffffeL), -EPERM);
        expect("root_uid_eperm", ch(D "/f", 0, -1), -EPERM);
        a_stat_path(D "/f", &b);
        check("ids_unchanged", b.st_uid == (u32)uid && b.st_gid == (u32)gid, b.st_uid);
        check("bits_unchanged", (b.st_mode & 07777) == 06755, b.st_mode & 07777);
        check("ctime_unchanged", a.st_ctime_sec == b.st_ctime_sec && a.st_ctime_nsec == b.st_ctime_nsec, 0);
    } else {
        expect("root_other_uid", ch(D "/f", other_uid, -1), 0);
        expect("root_other_gid", ch(D "/f", -1, other_gid), 0);
        a_stat_path(D "/f", &b);
        check("root_ids_set", b.st_uid == (u32)other_uid && b.st_gid == (u32)other_gid, b.st_uid);
        check("root_bits_cleared", (b.st_mode & 07777) == 0755, b.st_mode & 07777);
    }
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
