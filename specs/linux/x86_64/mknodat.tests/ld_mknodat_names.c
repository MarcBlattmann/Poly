/* mknodat name rules: a last component of 255 bytes works, 256 is
 * ENAMETOOLONG; a path through a symlink loop is ELOOP; the last
 * component is never followed (a dangling symlink, a loop or a symlink
 * to a directory there is EEXIST); a symlink in the directory part is
 * followed; "." and ".." are EEXIST; a FIFO used as a directory ENOTDIR.
 * A node created through a symlinked directory appears in its target.
 */
#include "a_sys.h"

#define SYS_mknodat 259
#define DIR "/tmp/ld_mknodat_names"

static char name[300];

static i64 mk(i64 d, const char *path) { return syscall4(SYS_mknodat, d, path, S_IFIFO | 0600, 0); }

static void cleanup(void) {
    a_unlink(DIR "/dangling");
    a_unlink(DIR "/loop");
    a_unlink(DIR "/dirlink");
    a_unlink(DIR "/fifo");
    a_unlink(DIR "/sub/via");
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_mkdir(DIR "/sub", 0755);
    syscall2(SYS_symlink, "nowhere", DIR "/dangling");
    syscall2(SYS_symlink, "loop", DIR "/loop");
    syscall2(SYS_symlink, "sub", DIR "/dirlink");
    i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
    expect("dirfd_3", d, 3);

    for (int i = 0; i < 255; i++) name[i] = 'z';
    expect("name_255_ok", mk(d, name), 0);
    expect("name_255_unlink", syscall3(SYS_unlinkat, d, name, 0), 0);
    name[255] = 'z';
    expect("name_256_enametoolong", mk(d, name), -ENAMETOOLONG);
    name[0] = 0;

    expect("loop_dir_part_eloop", mk(d, "loop/x"), -ELOOP);
    expect("loop_last_eexist", mk(d, "loop"), -EEXIST);
    expect("dangling_last_eexist", mk(d, "dangling"), -EEXIST);
    struct a_stat st;
    expect("dangling_target_absent", syscall4(SYS_newfstatat, d, "nowhere", &st, AT_SYMLINK_NOFOLLOW), -ENOENT);
    expect("dirlink_last_eexist", mk(d, "dirlink"), -EEXIST);
    expect("via_dirlink", mk(d, "dirlink/via"), 0);
    expect("via_in_target", a_lstat_path(DIR "/sub/via", &st), 0);
    expect("via_is_fifo", st.st_mode & S_IFMT, S_IFIFO);
    expect("dot_eexist", mk(d, "."), -EEXIST);
    expect("dotdot_eexist", mk(d, ".."), -EEXIST);
    expect("sub_dotdot_eexist", mk(d, "sub/.."), -EEXIST);
    expect("fifo", mk(d, "fifo"), 0);
    expect("fifo_as_dir_enotdir", mk(d, "fifo/x"), -ENOTDIR);

    a_close(d);
    cleanup();
    return finish();
}
