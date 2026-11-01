/* mknodat permission checks happen on the directory at call time; a
 * descriptor opened by someone else grants nothing. A caller without
 * write (or search) permission on the directory gets EACCES, also
 * through a dirfd opened while privileged and through an O_PATH dirfd of
 * a mode-0700 directory; EACCES comes before EPERM for a device node.
 * In a mode-0777 directory the unprivileged caller makes FIFOs (owned by
 * it) but not device nodes (EPERM). Root (CAP_DAC_OVERRIDE, CAP_MKNOD)
 * passes all of these.
 * poly-diff: needs=root (drops to uid/gid 65534 in a child)
 */
#include "a_sys.h"

#define SYS_mknodat 259
#define SYS_setresuid 117
#define SYS_setgroups 116
#define SYS_setresgid 119
#define DIR "/tmp/ld_mknodat_perm"

static i64 mk(i64 d, const char *path, u64 mode, u64 dev) { return syscall4(SYS_mknodat, d, path, mode, dev); }

static void cleanup(void) {
    a_unlink(DIR "/ro/root");
    a_unlink(DIR "/priv/root");
    a_unlink(DIR "/open/ufifo");
    a_unlink(DIR "/open/rootdev");
    a_rmdir(DIR "/ro");
    a_rmdir(DIR "/priv");
    a_rmdir(DIR "/open");
    a_rmdir(DIR);
}

static i64 ro, priv, open;

static int as_user(void) {
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 65534, 65534, 65534);
    expect("user_setresuid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
    expect("user_ro_dirfd_eacces", mk(ro, "u", S_IFIFO | 0600, 0), -EACCES);
    expect("user_ro_dirfd_device_eacces", mk(ro, "u", S_IFCHR | 0600, 0x103), -EACCES);
    expect("user_priv_opath_eacces", mk(priv, "u", S_IFIFO | 0600, 0), -EACCES);
    expect("user_priv_path_eacces", mk(AT_FDCWD, DIR "/priv/u", S_IFIFO | 0600, 0), -EACCES);
    expect("user_open_fifo_ok", mk(open, "ufifo", S_IFIFO | 0640, 0), 0);
    struct a_stat st;
    expect("user_fifo_stat", syscall4(SYS_newfstatat, open, "ufifo", &st, 0), 0);
    expect("user_fifo_uid", st.st_uid, 65534);
    expect("user_fifo_gid", st.st_gid, 65534);
    expect("user_open_device_eperm", mk(open, "udev", S_IFCHR | 0600, 0x103), -EPERM);
    expect("user_existing_eexist_first", mk(ro, "root", S_IFIFO | 0600, 0), -EEXIST);
    return failed;
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_mkdir(DIR "/ro", 0755);
    a_mkdir(DIR "/priv", 0700);
    a_mkdir(DIR "/open", 0777);
    syscall2(SYS_chmod, DIR "/open", 0777);
    ro = a_open(DIR "/ro", O_RDONLY | O_DIRECTORY, 0);
    priv = a_open(DIR "/priv", O_PATH | O_DIRECTORY, 0);
    open = a_open(DIR "/open", O_RDONLY | O_DIRECTORY, 0);
    expect("fds", ro == 3 && priv == 4 && open == 5, 1);
    expect("root_ro_ok", mk(ro, "root", S_IFIFO | 0600, 0), 0);
    expect("root_priv_ok", mk(priv, "root", S_IFIFO | 0600, 0), 0);
    expect("root_open_device_ok", mk(open, "rootdev", S_IFCHR | 0600, 0x103), 0);

    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(as_user() ? 1 : 0);
    i32 status = -1;
    expect("wait_child", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);
    cleanup();
    return finish();
}
