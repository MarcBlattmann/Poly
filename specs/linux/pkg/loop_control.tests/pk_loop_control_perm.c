/* Loop devices have no capability checks of their own: whoever may open /dev/loop-control
 * and a loop device node can allocate, add, remove, bind and unbind devices. A process
 * running as uid 65534 without capabilities, given nodes with mode 0666, binds a file it
 * can open to a free device and reads through the device; without access to the node
 * (mode 0600, owned by root) it cannot even open them (EACCES). Making the nodes themselves
 * (mknod of a block or character device) needs CAP_MKNOD (EPERM).
 * poly-diff: needs=root (mknod, setuid)
 * poly-diff: syscalls=off (child process, device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_lc_perm"
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_chmod 90

static void drop(void) {
    syscall3(SYS_setresgid, 65534, 65534, 65534);
    syscall3(SYS_setresuid, 65534, 65534, 65534);
}

int test_main(void) {
    a_block_sigchld();
    expect("mkdir", p_mkdir(D), 0);
    syscall2(SYS_chmod, D, 0777);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    char data[1024];
    for (int i = 0; i < 1024; i++) data[i] = (char)('A' + i % 26);
    i64 b = p_file(D "/b", data, sizeof data);
    expect("chmod_backing_0644", syscall2(SYS_chmod, D "/b", 0644), 0);
    i64 n = p_ioctl(ctl, LOOP_CTL_GET_FREE, 0);
    check("get_free", n >= 0, n);
    expect("mknod_node", p_node(D "/l", n), 0);
    /* first: nodes are 0600 root-only, the unprivileged child is refused */
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        drop();
        expect("child_uid", syscall0(SYS_getuid), 65534);
        expect("child_open_control_eacces", a_open(D "/ctl", O_RDWR, 0), -EACCES);
        expect("child_open_loop_eacces", a_open(D "/l", O_RDWR, 0), -EACCES);
        expect("child_mknod_block_eperm", syscall3(SYS_mknod, D "/mine", S_IFBLK | 0600, p_dev(LOOP_MAJOR, 3)), -EPERM);
        expect("child_mknod_char_eperm", syscall3(SYS_mknod, D "/mine", S_IFCHR | 0600, p_dev(10, 237)), -EPERM);
        a_shared()[0] = passed;
        a_shared()[1] = failed;
        syscall1(SYS_exit_group, 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child1_exit_0", status, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    /* now open to everybody */
    expect("chmod_control_0666", syscall2(SYS_chmod, D "/ctl", 0666), 0);
    expect("chmod_node_0666", syscall2(SYS_chmod, D "/l", 0666), 0);
    a_shared()[0] = 0;
    a_shared()[1] = 0;
    pid = a_fork_sync();
    if (pid == 0) {
        drop();
        i64 c = a_open(D "/ctl", O_RDWR, 0);
        check("child_open_control", c >= 0, c);
        check("child_get_free", p_ioctl(c, LOOP_CTL_GET_FREE, 0) >= 0, 0);
        i64 added = p_ioctl(c, LOOP_CTL_ADD, -1);
        check("child_add", added >= 0, added);
        if (added >= 0) expect("child_remove", p_ioctl(c, LOOP_CTL_REMOVE, added), 0);
        i64 bf = a_open(D "/b", O_RDONLY, 0);
        check("child_open_backing", bf >= 0, bf);
        i64 d = a_open(D "/l", O_RDWR, 0);
        check("child_open_loop", d >= 0, d);
        expect("child_set_fd", p_ioctl(d, LOOP_SET_FD, bf), 0);
        char got[16];
        memset(got, 0, sizeof got);
        expect("child_read_through_device", syscall4(SYS_pread64, d, got, 8, 10), 8);
        check("child_data_matches", mem_eq(got, data + 10, 8), 0);
        expect("child_clr_fd", p_ioctl(d, LOOP_CLR_FD, 0), 0);
        a_shared()[0] = passed;
        a_shared()[1] = failed;
        syscall1(SYS_exit_group, 0);
    }
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child2_exit_0", status, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    a_close(b);
    a_close(ctl);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/ctl");
    a_unlink(D "/mine");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
