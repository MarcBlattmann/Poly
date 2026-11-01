/* fchmodat2 errors and their order. flags other than
 * AT_SYMLINK_NOFOLLOW (0x100) and AT_EMPTY_PATH (0x1000) are EINVAL
 * (0x1, AT_REMOVEDIR 0x200, AT_SYMLINK_FOLLOW 0x400, AT_NO_AUTOMOUNT
 * 0x800, AT_STATX_FORCE_SYNC 0x2000), checked before the path is read
 * and before dirfd. Then the path (EFAULT, even with a bad dirfd), then
 * dirfd (EBADF; ENOTDIR for a file descriptor with a relative path),
 * then the lookup: ENOENT (missing, empty path, dangling symlink),
 * ENOTDIR, ELOOP, ENAMETOOLONG. Nothing changes on error.
 */
#include "a_sys.h"

#define SYS_fchmodat2 452
#define DIR "/tmp/ld_fchmodat2_errors"

static char longname[300];
static struct a_stat st;

static i64 chm(i64 d, const void *p, u64 mode, u64 flags) { return syscall4(SYS_fchmodat2, d, p, mode, flags); }

static void cleanup(void) {
    a_unlink(DIR "/f");
    a_unlink(DIR "/loop");
    a_unlink(DIR "/dl");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    expect("file", a_make_file(DIR "/f", "x", 0644), 0);
    syscall2(SYS_symlink, "loop", DIR "/loop");
    syscall2(SYS_symlink, "nowhere", DIR "/dl");
    i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
    expect("dirfd_3", d, 3);
    i64 f = a_open(DIR "/f", O_RDONLY, 0);
    expect("filefd_4", f, 4);

    expect("flag_1_einval", chm(d, "f", 0600, 0x1), -EINVAL);
    expect("flag_removedir_einval", chm(d, "f", 0600, AT_REMOVEDIR), -EINVAL);
    expect("flag_follow_einval", chm(d, "f", 0600, AT_SYMLINK_FOLLOW), -EINVAL);
    expect("flag_no_automount_einval", chm(d, "f", 0600, AT_NO_AUTOMOUNT), -EINVAL);
    expect("flag_force_sync_einval", chm(d, "f", 0600, AT_STATX_FORCE_SYNC), -EINVAL);
    expect("flag_bit_31_einval", chm(d, "f", 0600, 0x80000000UL), -EINVAL);
    expect("valid_plus_bad_einval", chm(d, "f", 0600, AT_SYMLINK_NOFOLLOW | 0x1), -EINVAL);
    expect("bad_flags_null_path_einval", chm(d, 0, 0600, 0x1), -EINVAL);
    expect("bad_flags_bad_fd_einval", chm(99, "f", 0600, 0x1), -EINVAL);
    expect("mode_unchanged", (a_stat_path(DIR "/f", &st), st.st_mode), S_IFREG | 0644);

    expect("null_path_efault", chm(d, 0, 0600, 0), -EFAULT);
    expect("null_path_bad_fd_efault", chm(99, 0, 0600, 0), -EFAULT);
    expect("bad_fd_ebadf", chm(99, "f", 0600, 0), -EBADF);
    expect("negative_fd_ebadf", chm(-1, "f", 0600, 0), -EBADF);
    expect("file_dirfd_enotdir", chm(f, "f", 0600, 0), -ENOTDIR);
    expect("missing_enoent", chm(d, "nope", 0600, 0), -ENOENT);
    expect("empty_enoent", chm(d, "", 0600, 0), -ENOENT);
    expect("dangling_enoent", chm(d, "dl", 0600, 0), -ENOENT);
    expect("file_component_enotdir", chm(d, "f/x", 0600, 0), -ENOTDIR);
    expect("trailing_slash_file_enotdir", chm(d, "f/", 0600, 0), -ENOTDIR);
    expect("loop_eloop", chm(d, "loop", 0600, 0), -ELOOP);
    expect("loop_dir_part_eloop", chm(d, "loop/x", 0600, AT_SYMLINK_NOFOLLOW), -ELOOP);
    for (int i = 0; i < 256; i++) longname[i] = 'q';
    expect("name_256_enametoolong", chm(d, longname, 0600, 0), -ENAMETOOLONG);
    expect("still_unchanged", (a_stat_path(DIR "/f", &st), st.st_mode), S_IFREG | 0644);

    a_close(d);
    a_close(f);
    cleanup();
    return finish();
}
