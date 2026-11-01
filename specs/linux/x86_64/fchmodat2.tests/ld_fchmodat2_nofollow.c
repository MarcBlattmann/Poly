/* fchmodat2 with AT_SYMLINK_NOFOLLOW (0x100): a last component that is
 * not a symlink is changed as usual; a symlink (also a dangling one) is
 * not followed and its mode cannot be changed: EOPNOTSUPP, and neither
 * the link nor its target changes. (The older fchmodat, syscall 268, has
 * no flags argument: a fourth argument is ignored and the symlink is
 * followed.) A symlink in the directory part is still followed.
 */
#include "a_sys.h"

#define SYS_fchmodat2 452
#define DIR "/tmp/ld_fchmodat2_nofollow"
#define EOPNOTSUPP_ 95

static struct a_stat st;

static u64 mode_of(const char *p) {
    memset(&st, 0, sizeof st);
    a_lstat_path(p, &st);
    return st.st_mode;
}

static i64 chm(i64 d, const char *p, u64 mode, u64 flags) { return syscall4(SYS_fchmodat2, d, p, mode, flags); }

static void cleanup(void) {
    a_unlink(DIR "/f");
    a_unlink(DIR "/l");
    a_unlink(DIR "/dl");
    a_unlink(DIR "/sublink");
    a_unlink(DIR "/sub/g");
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_mkdir(DIR "/sub", 0755);
    expect("file", a_make_file(DIR "/f", "x", 0644), 0);
    a_make_file(DIR "/sub/g", "y", 0644);
    syscall2(SYS_symlink, "f", DIR "/l");
    syscall2(SYS_symlink, "nowhere", DIR "/dl");
    syscall2(SYS_symlink, "sub", DIR "/sublink");
    i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
    expect("dirfd_3", d, 3);

    expect("nofollow_regular", chm(d, "f", 0600, AT_SYMLINK_NOFOLLOW), 0);
    expect("nofollow_regular_mode", mode_of(DIR "/f"), S_IFREG | 0600);
    expect("nofollow_dir", chm(d, "sub", 0700, AT_SYMLINK_NOFOLLOW), 0);
    expect("nofollow_dir_mode", mode_of(DIR "/sub"), S_IFDIR | 0700);
    expect("nofollow_symlink_eopnotsupp", chm(d, "l", 0640, AT_SYMLINK_NOFOLLOW), -EOPNOTSUPP_);
    expect("link_mode_unchanged", mode_of(DIR "/l"), S_IFLNK | 0777);
    expect("target_unchanged", mode_of(DIR "/f"), S_IFREG | 0600);
    expect("nofollow_dangling_eopnotsupp", chm(d, "dl", 0640, AT_SYMLINK_NOFOLLOW), -EOPNOTSUPP_);
    expect("follow_dangling_enoent", chm(d, "dl", 0640, 0), -ENOENT);
    expect("nofollow_dir_part_followed", chm(d, "sublink/g", 0400, AT_SYMLINK_NOFOLLOW), 0);
    expect("dir_part_target_mode", mode_of(DIR "/sub/g"), S_IFREG | 0400);
    expect("nofollow_dir_symlink_eopnotsupp", chm(d, "sublink", 0755, AT_SYMLINK_NOFOLLOW), -EOPNOTSUPP_);
    expect("sub_unchanged", mode_of(DIR "/sub"), S_IFDIR | 0700);
    expect("follow_dir_symlink", chm(d, "sublink", 0755, 0), 0);
    expect("sub_changed", mode_of(DIR "/sub"), S_IFDIR | 0755);
    expect("nofollow_missing_enoent", chm(d, "nope", 0600, AT_SYMLINK_NOFOLLOW), -ENOENT);

    /* fchmodat (268) ignores a fourth argument and follows. */
    expect("fchmodat_ignores_flags", syscall4(SYS_fchmodat, d, "l", 0604, AT_SYMLINK_NOFOLLOW), 0);
    expect("fchmodat_changed_target", mode_of(DIR "/f"), S_IFREG | 0604);
    expect("fchmodat_link_unchanged", mode_of(DIR "/l"), S_IFLNK | 0777);

    a_close(d);
    cleanup();
    return finish();
}
