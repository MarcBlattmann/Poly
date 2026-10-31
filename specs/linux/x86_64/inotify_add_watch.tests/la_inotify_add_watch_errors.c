/* inotify_add_watch errors and their order: the mask is checked first
 * (no known bit at all, i.e. 0 in the low 32 bits; a bit inotify does
 * not know such as 0x1000, 0x10000 or 0x800000; or IN_MASK_ADD together
 * with IN_MASK_CREATE: EINVAL, even with a bad descriptor or path; a
 * mask of flag bits only, such as IN_ONLYDIR | IN_MASK_ADD, is accepted
 * and makes a watch that reports nothing); then the descriptor
 * (EBADF if not open, EINVAL if not an inotify instance); then the path
 * (EFAULT if unreadable, ENOENT for "" or a missing name, ENOTDIR for
 * IN_ONLYDIR on a non-directory or a file used as a directory, ELOOP for
 * a symlink loop, ENAMETOOLONG for a component over 255 bytes); then
 * IN_MASK_CREATE on an inode already watched by this instance: EEXIST.
 * The mask is 32 bits wide: upper bits are ignored.
 */
#include "a_sys.h"

#define SYS_inotify_add_watch 254
#define SYS_inotify_init1 294
/* linux/inotify.h */
#define IN_MODIFY 0x2
#define IN_CREATE 0x100
#define IN_ONLYDIR 0x01000000
#define IN_DONT_FOLLOW 0x02000000
#define IN_MASK_CREATE 0x10000000
#define IN_MASK_ADD 0x20000000

#define DIR "/tmp/la_inotify_add_watch_errors"

static i64 add(i64 fd, const char *path, u64 mask) { return syscall3(SYS_inotify_add_watch, fd, path, mask); }
static char long_name[300];

int test_main(void) {
    a_unlink(DIR "/file");
    a_unlink(DIR "/loop");
    a_rmdir(DIR);
    a_mkdir(DIR, 0755);
    a_make_file(DIR "/file", "x", 0644);
    syscall3(SYS_symlinkat, DIR "/loop", AT_FDCWD, DIR "/loop");

    i64 fd = syscall1(SYS_inotify_init1, 0);
    i64 reg = a_open(DIR "/file", O_RDONLY, 0);
    expect("fds", fd * 10 + reg, 34);

    /* Mask first. */
    expect("mask_zero_einval", add(fd, DIR, 0), -EINVAL);
    expect("mask_only_flag_bits_accepted", add(fd, DIR, IN_ONLYDIR | IN_MASK_ADD), 1);
    expect("mask_bit_0x1000_einval", add(fd, DIR, IN_CREATE | 0x1000), -EINVAL);
    expect("mask_bit_0x10000_einval", add(fd, DIR, IN_CREATE | 0x10000), -EINVAL);
    expect("mask_bit_0x800000_einval", add(fd, DIR, IN_CREATE | 0x800000), -EINVAL);
    expect("mask_add_and_create_einval", add(fd, DIR, IN_CREATE | IN_MASK_ADD | IN_MASK_CREATE), -EINVAL);
    expect("mask_zero_beats_ebadf", add(999, DIR, 0), -EINVAL);
    expect("mask_zero_beats_efault", add(fd, 0, 0), -EINVAL);
    expect("mask_upper_only_einval", add(fd, DIR, 1UL << 40), -EINVAL);

    /* Then the descriptor. */
    expect("bad_fd_ebadf", add(999, DIR, IN_CREATE), -EBADF);
    expect("negative_fd_ebadf", add(-1, DIR, IN_CREATE), -EBADF);
    expect("bad_fd_beats_efault", add(999, 0, IN_CREATE), -EBADF);
    expect("not_inotify_einval", add(reg, DIR, IN_CREATE), -EINVAL);
    expect("not_inotify_beats_enoent", add(reg, "/nonexistent", IN_CREATE), -EINVAL);

    /* Then the path. */
    expect("null_path_efault", add(fd, 0, IN_CREATE), -EFAULT);
    expect("empty_path_enoent", add(fd, "", IN_CREATE), -ENOENT);
    expect("missing_enoent", add(fd, DIR "/missing", IN_CREATE), -ENOENT);
    expect("onlydir_on_file_enotdir", add(fd, DIR "/file", IN_CREATE | IN_ONLYDIR), -ENOTDIR);
    expect("file_as_dir_enotdir", add(fd, DIR "/file/x", IN_CREATE), -ENOTDIR);
    expect("symlink_loop_eloop", add(fd, DIR "/loop", IN_CREATE), -ELOOP);
    memset(long_name, 'a', sizeof long_name);
    long_name[0] = '/';
    long_name[290] = 0;
    expect("long_component_enametoolong", add(fd, long_name, IN_CREATE), -ENAMETOOLONG);

    /* Watching works; IN_MASK_CREATE on it again is EEXIST. */
    expect("watch_dir", add(fd, DIR, IN_CREATE), 1);
    expect("mask_create_existing_eexist", add(fd, DIR, IN_CREATE | IN_MASK_CREATE), -EEXIST);
    expect("mask_create_new_ok", add(fd, DIR "/file", IN_MODIFY | IN_MASK_CREATE), 2);
    expect("dont_follow_loop_ok", add(fd, DIR "/loop", IN_CREATE | IN_DONT_FOLLOW), 3);
    expect("upper_mask_bits_ignored", add(fd, DIR, (1UL << 40) | IN_CREATE), 1);

    a_unlink(DIR "/file");
    a_unlink(DIR "/loop");
    a_rmdir(DIR);
    return finish();
}
