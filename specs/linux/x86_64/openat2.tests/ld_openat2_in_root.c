/* openat2 with RESOLVE_IN_ROOT (0x10): dirfd's directory acts as the
 * root for this one resolution, as if the caller had chrooted there.
 * Absolute paths start at dirfd's directory, ".." at it stays there,
 * absolute symlink targets are resolved inside it, and symlinks that
 * climb are clamped (so "sub/esc/f", with esc -> "../../..", is dirfd's
 * "f"). A symlink to "/tmp" means dirfd's "tmp" (ENOENT here). O_CREAT
 * of "/c" creates "c" in dirfd's directory.
 */
#include "a_sys.h"

#define SYS_openat2 437
#define DIR "/tmp/ld_openat2_in_root"
#define RESOLVE_IN_ROOT 0x10

/* linux/openat2.h */
struct open_how {
    u64 flags, mode, resolve;
};
static struct open_how how;

static i64 inr(i64 d, const char *p, u64 flags, u64 mode) {
    how.flags = flags;
    how.mode = mode;
    how.resolve = RESOLVE_IN_ROOT;
    return syscall4(SYS_openat2, d, p, &how, sizeof how);
}

/* Opens p in root d; checks it is the file whose contents are `want`. */
static void same(const char *name, i64 d, const char *p, const char *want) {
    i64 fd = inr(d, p, O_RDONLY, 0);
    char buf[16];
    memset(buf, 0, sizeof buf);
    i64 n = fd >= 0 ? a_read(fd, buf, sizeof buf - 1) : fd;
    check(name, n == (i64)str_len(want) && mem_eq(buf, want, n), n);
    if (fd >= 0) a_close(fd);
}

static void cleanup(void) {
    a_unlink(DIR "/f");
    a_unlink(DIR "/c");
    a_unlink(DIR "/al");
    a_unlink(DIR "/sub/f");
    a_unlink(DIR "/sub/absf");
    a_unlink(DIR "/sub/esc");
    a_unlink(DIR "/sub/root");
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_mkdir(DIR "/sub", 0755);
    expect("top_file", a_make_file(DIR "/f", "top", 0644), 0);
    expect("sub_file", a_make_file(DIR "/sub/f", "sub", 0644), 0);
    syscall2(SYS_symlink, "/tmp", DIR "/al");
    syscall2(SYS_symlink, "/f", DIR "/sub/absf");
    syscall2(SYS_symlink, "../../..", DIR "/sub/esc");
    syscall2(SYS_symlink, "/", DIR "/sub/root");
    i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
    expect("dirfd_3", d, 3);

    same("relative", d, "f", "top");
    same("absolute_from_root", d, "/f", "top");
    same("absolute_sub", d, "/sub/f", "sub");
    same("dotdot_clamped", d, "../../f", "top");
    same("dotdot_in_middle", d, "sub/../../sub/f", "sub");
    same("absolute_symlink_inside", d, "sub/absf", "top");
    same("climbing_symlink_clamped", d, "sub/esc/f", "top");
    same("symlink_to_slash", d, "sub/root/sub/f", "sub");
    expect("symlink_to_tmp_enoent", inr(d, "al", O_RDONLY, 0), -ENOENT);
    expect("absolute_missing_enoent", inr(d, "/tmp", O_RDONLY, 0), -ENOENT);

    i64 r = inr(d, "/", O_RDONLY | O_DIRECTORY, 0);
    expect("slash_is_dirfd_dir", r, 4);
    struct a_stat a, b;
    syscall2(SYS_fstat, r, &a);
    syscall2(SYS_fstat, d, &b);
    check("slash_same_inode", a.st_ino == b.st_ino && a.st_dev == b.st_dev, 0);
    a_close(r);

    i64 c = inr(d, "/c", O_CREAT | O_WRONLY, 0600);
    expect("creat_absolute", c, 4);
    a_close(c);
    expect("created_in_dirfd_dir", a_stat_path(DIR "/c", &a), 0);
    expect("created_mode", a.st_mode, S_IFREG | 0600);

    /* Rooted at a subdirectory. */
    i64 s = a_open(DIR "/sub", O_PATH | O_DIRECTORY, 0);
    expect("subdirfd_4", s, 4);
    same("sub_root_absolute", s, "/f", "sub");
    same("sub_root_dotdot", s, "../f", "sub");
    same("sub_root_absolute_symlink", s, "absf", "sub");
    a_close(s);

    a_close(d);
    cleanup();
    return finish();
}
