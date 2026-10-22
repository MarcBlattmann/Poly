/* chown needs search (execute) permission on every directory of the path,
 * nothing else: without it a caller that is not root gets EACCES, even
 * for a missing name. Write permission on the parent and any permission
 * on the file itself do not matter. Root passes the search check. */
#include "a_sys.h"

#define D "/tmp/c_chown_search_permission"

static i64 ch(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/ns", 0755), 0);
    expect("setup_file", a_make_file(D "/ns/g", "", 0644), 0);
    syscall2(SYS_chmod, D "/ns", 0644);
    if (uid != 0) {
        expect("no_search_eacces", ch(D "/ns/g", -1, -1), -EACCES);
        expect("no_search_missing_eacces", ch(D "/ns/zz", -1, -1), -EACCES);
        expect("no_search_deeper_eacces", ch(D "/ns/zz/y", -1, -1), -EACCES);
    } else {
        expect("root_no_search_ok", ch(D "/ns/g", -1, -1), 0);
        expect("root_no_search_missing_enoent", ch(D "/ns/zz", -1, -1), -ENOENT);
        expect("root_no_search_deeper_enoent", ch(D "/ns/zz/y", -1, -1), -ENOENT);
    }
    expect("dir_itself_without_search", ch(D "/ns", -1, -1), 0);
    syscall2(SYS_chmod, D "/ns", 0111);
    expect("search_only_dir", ch(D "/ns/g", uid, gid), 0);
    syscall2(SYS_chmod, D "/ns", 0555);
    expect("read_only_parent", ch(D "/ns/g", uid, gid), 0);
    syscall2(SYS_chmod, D "/ns/g", 0);
    expect("mode_zero_file", ch(D "/ns/g", uid, gid), 0);
    syscall2(SYS_chmod, D "/ns", 0755);
    expect("cleanup_g", a_unlink(D "/ns/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/ns"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
