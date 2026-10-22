/* chown of a file the caller does not own: -1 for both ids succeeds for
 * any caller (here on "/", owned by root); a caller that is not root and
 * not the owner gets EPERM for any id it names, even its own. */
#include "a_sys.h"

static i64 ch(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }

int test_main(void) {
    struct a_stat st;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    expect("stat_root", a_stat_path("/", &st), 0);
    expect("root_dir_noop", ch("/", -1, -1), 0);
    expect("root_dir_ffffffff", ch("/", 0xffffffffL, 0xffffffffL), 0);
    if (uid != 0 && st.st_uid != (u32)uid) {
        expect("own_uid_eperm", ch("/", uid, -1), -EPERM);
        expect("own_gid_eperm", ch("/", -1, gid), -EPERM);
        expect("current_owner_eperm", ch("/", st.st_uid, -1), -EPERM);
        expect("current_group_eperm", ch("/", -1, st.st_gid), -EPERM);
    }
    struct a_stat after;
    a_stat_path("/", &after);
    check("owner_unchanged", after.st_uid == st.st_uid && after.st_gid == st.st_gid, after.st_uid);
    check("mode_unchanged", after.st_mode == st.st_mode, after.st_mode);
    return finish();
}
