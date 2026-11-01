/* mknod of character and block device nodes needs CAP_MKNOD. dev is a
 * 32-bit value (bits 32..63 ignored) in the kernel's "new" encoding:
 * major = bits 8..19, minor = bits 0..7 plus bits 20..31 as minor bits
 * 8..19; stat's st_rdev shows the same number for these values
 * (0x103 = 1:3, 0xfff00 = 4095:0, 0xfff000ff = 0:1048575).
 * Without CAP_MKNOD (here: a child that dropped to uid/gid 65534) a
 * device node is EPERM, checked after the lookup and the directory's
 * write permission: a missing directory is ENOENT, an existing name
 * EEXIST, a directory the caller may not write EACCES. FIFOs still work
 * for such a caller.
 * poly-diff: needs=root (creating device nodes and dropping to another uid)
 */
#include "a_sys.h"

#define SYS_mknod 133
#define SYS_setresuid 117
#define SYS_setgroups 116
#define SYS_setresgid 119
#define DIR "/tmp/ld_mknod_device"

static struct a_stat st;

static i64 mk(const char *p, u64 mode, u64 dev) { return syscall3(SYS_mknod, p, mode, dev); }

static void cleanup(void) {
    static const char *names[] = {DIR "/c", DIR "/b", DIR "/big", DIR "/minor", DIR "/high", DIR "/open/fifo",
                                  DIR "/open/c", 0};
    for (int i = 0; names[i]; i++) a_unlink(names[i]);
    a_rmdir(DIR "/open");
    a_rmdir(DIR);
}

static int as_user(void) {
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 65534, 65534, 65534);
    expect("user_setresuid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
    expect("user_chr_eperm", mk(DIR "/open/c", S_IFCHR | 0600, 0x103), -EPERM);
    expect("user_blk_eperm", mk(DIR "/open/c", S_IFBLK | 0600, 0x700), -EPERM);
    expect("user_chr_missing_dir_enoent", mk(DIR "/none/c", S_IFCHR | 0600, 0x103), -ENOENT);
    expect("user_chr_existing_eexist", mk(DIR "/c", S_IFCHR | 0600, 0x103), -EEXIST);
    expect("user_chr_unwritable_dir_eacces", mk(DIR "/c2", S_IFCHR | 0600, 0x103), -EACCES);
    expect("user_fifo_unwritable_dir_eacces", mk(DIR "/f2", S_IFIFO | 0600, 0), -EACCES);
    expect("user_fifo_ok", mk(DIR "/open/fifo", S_IFIFO | 0600, 0), 0);
    expect("user_fifo_owner", (a_stat_path(DIR "/open/fifo", &st), st.st_uid), 65534);
    return failed;
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    expect("mkdir_open", a_mkdir(DIR "/open", 0777), 0);
    syscall2(SYS_chmod, DIR "/open", 0777);

    expect("chr", mk(DIR "/c", S_IFCHR | 0666, 0x0103), 0);
    a_stat_path(DIR "/c", &st);
    expect("chr_mode_umask", st.st_mode, S_IFCHR | 0644);
    expect("chr_rdev_1_3", st.st_rdev, 0x0103);
    expect("chr_size_0", st.st_size, 0);
    expect("blk", mk(DIR "/b", S_IFBLK | 0600, 0x0700), 0);
    a_stat_path(DIR "/b", &st);
    expect("blk_mode", st.st_mode, S_IFBLK | 0600);
    expect("blk_rdev_7_0", st.st_rdev, 0x0700);
    expect("big_major", mk(DIR "/big", S_IFCHR | 0600, 0xfff00), 0);
    a_stat_path(DIR "/big", &st);
    expect("big_major_rdev", st.st_rdev, 0xfff00);
    expect("big_minor", mk(DIR "/minor", S_IFCHR | 0600, 0xfff000ffUL), 0);
    a_stat_path(DIR "/minor", &st);
    expect("big_minor_rdev", st.st_rdev, 0xfff000ffUL);
    expect("dev_bits_32_63_ignored", mk(DIR "/high", S_IFCHR | 0600, (1UL << 32) | 0x0105), 0);
    a_stat_path(DIR "/high", &st);
    expect("dev_high_rdev", st.st_rdev, 0x0105);

    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(as_user() ? 1 : 0);
    i32 status = -1;
    expect("wait_child", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    cleanup();
    return finish();
}
