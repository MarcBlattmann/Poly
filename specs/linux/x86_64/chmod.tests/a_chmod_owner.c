/* chmod: only the owner (or root) may change a mode. A file the caller
 * owns can be changed whatever its own mode and whatever the mode of its
 * directory (search permission aside); a file owned by someone else, such
 * as "/" or "/tmp" for a normal user, is EPERM and keeps its mode, even
 * when the new mode equals the old. The expectations depend on the uid;
 * the check names do not. */
#include "a_sys.h"

#define D "/tmp/a_chmod_owner"

static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_stat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    i64 euid = syscall0(SYS_geteuid);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0000), 0);
    expect("own_mode_0_chmod", syscall2(SYS_chmod, D "/f", 0644), 0);
    expect("own_mode_0_result", mode_of(D "/f"), S_IFREG | 0644);
    expect("dir_0500", syscall2(SYS_chmod, D, 0500), 0);
    expect("in_readonly_dir_chmod", syscall2(SYS_chmod, D "/f", 0600), 0);
    expect("in_readonly_dir_result", mode_of(D "/f"), S_IFREG | 0600);
    expect("dir_0100", syscall2(SYS_chmod, D, 0100), 0);
    expect("search_only_dir_chmod", syscall2(SYS_chmod, D "/f", 0640), 0);
    expect("search_only_dir_result", mode_of(D "/f"), S_IFREG | 0640);
    expect("dir_0000", syscall2(SYS_chmod, D, 0000), 0);
    expect("dir_0000_result", mode_of(D), S_IFDIR | 0000);
    expect("dir_back_0755", syscall2(SYS_chmod, D, 0755), 0);

    const char *others[] = {"/", "/tmp", "/tmp/", "/tmp/."};
    const char *names[] = {"root_dir", "tmp_dir", "tmp_slash", "tmp_dot"};
    for (int i = 0; i < 4; i++) {
        struct a_stat st;
        a_stat_path(others[i], &st);
        i64 want = (euid == 0 || st.st_uid == (u64)euid) ? 0 : -EPERM;
        /* The mode it already has: harmless where the call succeeds. */
        expect(names[i], syscall2(SYS_chmod, others[i], st.st_mode & 07777), want);
        struct a_stat after;
        a_stat_path(others[i], &after);
        check("mode_kept", after.st_mode == st.st_mode, after.st_mode);
    }
    /* A different mode: only tried where it must fail. */
    struct a_stat st, after;
    a_stat_path("/tmp", &st);
    if (euid == 0 || st.st_uid == (u64)euid) {
        check("tmp_other_mode_eperm", 1, 0);
    } else {
        expect("tmp_other_mode_eperm", syscall2(SYS_chmod, "/tmp", 0700), -EPERM);
    }
    a_stat_path("/tmp", &after);
    check("tmp_mode_kept", after.st_mode == st.st_mode, after.st_mode);

    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
