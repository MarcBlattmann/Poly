/* openat2 resolve flags that refuse kinds of links and mounts:
 *   - RESOLVE_NO_SYMLINKS (0x04): any symlink met on the way, in the
 *     middle or as the last component, is ELOOP (also with O_NOFOLLOW;
 *     but O_PATH | O_NOFOLLOW opens the last-component symlink itself),
 *     and O_CREAT through a dangling symlink is ELOOP (without the flag it
 *     creates the target);
 *   - RESOLVE_NO_MAGICLINKS (0x02): /proc/self/fd/N and similar
 *     "magic" links are ELOOP (O_PATH | O_NOFOLLOW opens the link itself);
 *     ordinary symlinks still work; RESOLVE_NO_SYMLINKS refuses magic
 *     links too;
 *   - RESOLVE_NO_XDEV (0x01): crossing a mount point (into /proc) is
 *     EXDEV; staying on one file system works.
 */
#include "a_sys.h"

#define SYS_openat2 437
#define DIR "/tmp/ld_openat2_nosymlinks"
#define RESOLVE_NO_XDEV 0x01
#define RESOLVE_NO_MAGICLINKS 0x02
#define RESOLVE_NO_SYMLINKS 0x04

/* linux/openat2.h */
struct open_how {
    u64 flags, mode, resolve;
};
static struct open_how how;

static i64 o2(i64 d, const char *p, u64 flags, u64 mode, u64 resolve) {
    how.flags = flags;
    how.mode = mode;
    how.resolve = resolve;
    return syscall4(SYS_openat2, d, p, &how, sizeof how);
}

static void good(const char *name, i64 fd, i64 want) {
    expect(name, fd, want);
    if (fd >= 0) a_close(fd);
}

static void cleanup(void) {
    a_unlink(DIR "/f");
    a_unlink(DIR "/rl");
    a_unlink(DIR "/dl");
    a_unlink(DIR "/target");
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
    syscall2(SYS_symlink, "f", DIR "/rl");
    syscall2(SYS_symlink, "target", DIR "/dl");
    syscall2(SYS_symlink, "sub", DIR "/sublink");
    i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
    expect("dirfd_3", d, 3);

    good("nosym_plain_file", o2(d, "f", O_RDONLY, 0, RESOLVE_NO_SYMLINKS), 4);
    good("nosym_absolute_without_links", o2(AT_FDCWD, DIR "/sub/g", O_RDONLY, 0, RESOLVE_NO_SYMLINKS), 4);
    expect("nosym_last_eloop", o2(d, "rl", O_RDONLY, 0, RESOLVE_NO_SYMLINKS), -ELOOP);
    expect("nosym_middle_eloop", o2(d, "sublink/g", O_RDONLY, 0, RESOLVE_NO_SYMLINKS), -ELOOP);
    expect("nosym_nofollow_eloop", o2(d, "rl", O_RDONLY | O_NOFOLLOW, 0, RESOLVE_NO_SYMLINKS), -ELOOP);
    i64 l = o2(d, "rl", O_PATH | O_NOFOLLOW, 0, RESOLVE_NO_SYMLINKS);
    expect("nosym_opath_nofollow_opens_link", l, 4);
    struct a_stat st;
    syscall2(SYS_fstat, l, &st);
    expect("nosym_opath_is_symlink", st.st_mode & S_IFMT, S_IFLNK);
    a_close(l);
    expect("nosym_creat_dangling_eloop", o2(d, "dl", O_CREAT | O_WRONLY, 0600, RESOLVE_NO_SYMLINKS), -ELOOP);
    expect("target_not_created", syscall4(SYS_newfstatat, d, "target", &st, 0), -ENOENT);
    good("creat_dangling_without_flag", o2(d, "dl", O_CREAT | O_WRONLY, 0600, 0), 4);
    expect("target_created", syscall4(SYS_newfstatat, d, "target", &st, 0), 0);

    /* Magic links: /proc/self/fd/3 refers to the directory. */
    good("magic_allowed_by_default", o2(AT_FDCWD, "/proc/self/fd/3", O_RDONLY | O_DIRECTORY, 0, 0), 4);
    expect("nomagic_eloop", o2(AT_FDCWD, "/proc/self/fd/3", O_RDONLY, 0, RESOLVE_NO_MAGICLINKS), -ELOOP);
    expect("nomagic_middle_eloop", o2(AT_FDCWD, "/proc/self/fd/3/f", O_RDONLY, 0, RESOLVE_NO_MAGICLINKS),
           -ELOOP);
    good("nomagic_opath_nofollow_link_itself",
         o2(AT_FDCWD, "/proc/self/fd/3", O_PATH | O_NOFOLLOW, 0, RESOLVE_NO_MAGICLINKS), 4);
    good("nomagic_ordinary_symlink_ok", o2(d, "rl", O_RDONLY, 0, RESOLVE_NO_MAGICLINKS), 4);
    expect("nosym_refuses_magic", o2(AT_FDCWD, "/proc/self/fd/3", O_RDONLY, 0, RESOLVE_NO_SYMLINKS), -ELOOP);

    /* Mount crossing. */
    good("noxdev_same_fs", o2(d, "sub/g", O_RDONLY, 0, RESOLVE_NO_XDEV), 4);
    good("noxdev_symlink_same_fs", o2(d, "sublink/g", O_RDONLY, 0, RESOLVE_NO_XDEV), 4);
    expect("noxdev_into_proc_exdev", o2(AT_FDCWD, "/proc/self", O_PATH, 0, RESOLVE_NO_XDEV), -EXDEV);
    i64 p = a_open("/proc", O_PATH | O_DIRECTORY, 0);
    expect("proc_fd_4", p, 4);
    expect("noxdev_out_of_proc_exdev", o2(p, "..", O_PATH, 0, RESOLVE_NO_XDEV), -EXDEV);
    good("noxdev_inside_proc", o2(p, "self", O_PATH, 0, RESOLVE_NO_XDEV), 5);
    a_close(p);

    a_close(d);
    cleanup();
    return finish();
}
