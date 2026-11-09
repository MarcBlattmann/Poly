/* Propagation between mount namespaces. A mount namespace made with
 * unshare(CLONE_NEWNS) is a copy of the mount table; a shared mount in it is
 * a peer of the original, so mounts and umounts below it appear in both
 * namespaces. If the child makes its copy a slave (MS_SLAVE) the parent's
 * events still reach it but its own do not reach the parent; a private mount
 * (MS_PRIVATE, or never shared) propagates nothing in either direction.
 * Four scenarios, each with its own tmpfs: shared/shared, shared then the
 * child makes it slave, shared then the child makes it private, and never
 * shared. The processes take turns through pipes, so the output is the same
 * on every run.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (two processes, mount namespaces)
 */
#include "p_mnt.h"

#define B "/tmp/pk_mpn"

static const char *g_tag;
static u64 g_ptype;   /* propagation type of the tmpfs in the parent, 0 = leave it private */
static u64 g_ctype;   /* applied by the child after unshare, 0 = leave it */
static int g_p2c;     /* parent's mount reaches the child */
static int g_c2p;     /* child's mount reaches the parent */


static void expect_flag(const char *what, int got, int want) {
    char buf[96];
    str_cpy(str_cpy(str_cpy(buf, g_tag), "_"), what);
    check(buf, got == want, got);
}

static void token(i64 fd, char c) { a_write(fd, &c, 1); }
static char wait_token(i64 fd) {
    char c = 0;
    a_read(fd, &c, 1);
    return c;
}

static int scenario(void) {
    const char *T = B "/t";
    expect("tmpfs", pm_tmpfs(T), 0);
    pm_mkdirs(B "/t/a");
    pm_mkdirs(B "/t/b");
    pm_mkdirs(B "/t/c");
    pm_mkdirs(B "/t/d");
    if (g_ptype) expect("set_type", pm_mount(0, T, 0, g_ptype, 0), 0);
    /* mounts that exist before the namespace is copied */
    expect("premount_a", pm_mount("pka", B "/t/a", "tmpfs", 0, 0), 0);
    pm_put(B, "/t/a/afile", "a", 0644);
    expect("premount_d", pm_mount("pkd", B "/t/d", "tmpfs", 0, 0), 0);
    pm_put(B, "/t/d/dfile", "d", 0644);
    i32 p2c[2], c2p[2];
    syscall2(SYS_pipe2, p2c, 0);
    syscall2(SYS_pipe2, c2p, 0);
    volatile i32 *sh = a_shared();
    i64 pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        sh[4] = 0;
        sh[5] = 0;
        expect("child_unshare", syscall1(SYS_unshare, CLONE_NEWNS), 0);
        if (g_ctype) expect("child_set_type", pm_mount(0, T, 0, g_ctype, 0), 0);
        expect_flag("child_has_copy_of_a", pm_exists(B "/t/a/afile"), 1);
        token(c2p[1], 'U');
        wait_token(p2c[0]); /* parent mounted c, unmounted a */
        expect_flag("child_sees_parent_new_mount", pm_exists(B "/t/c/cfile"), g_p2c);
        expect_flag("child_sees_parent_umount", pm_exists(B "/t/a/afile"), !g_p2c);
        expect("child_mount_b", pm_mount("pkchild", B "/t/b", "tmpfs", 0, 0), 0);
        pm_put(B, "/t/b/bfile", "b", 0644);
        expect("child_umount_d", pm_umount(B "/t/d", 0), 0);
        expect_flag("child_d_gone", pm_exists(B "/t/d/dfile"), 0);
        token(c2p[1], 'M');
        wait_token(p2c[0]);
        sh[4] = passed;
        sh[5] = failed;
        syscall1(SYS_exit_group, 0);
    }
    wait_token(c2p[0]); /* child unshared */
    expect("parent_mount_c", pm_mount("pkparent", B "/t/c", "tmpfs", 0, 0), 0);
    pm_put(B, "/t/c/cfile", "c", 0644);
    expect("parent_umount_a", pm_umount(B "/t/a", 0), 0);
    token(p2c[1], 'A');
    wait_token(c2p[0]); /* child mounted b, unmounted d */
    expect_flag("parent_sees_child_new_mount", pm_exists(B "/t/b/bfile"), g_c2p);
    expect_flag("parent_sees_child_umount", pm_exists(B "/t/d/dfile"), !g_c2p);
    token(p2c[1], 'B');
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += sh[4];
    failed += sh[5];
    return 0;
}

static void run(const char *tag, u64 ptype, u64 ctype, int p2c, int c2p) {
    g_tag = tag;
    g_ptype = ptype;
    g_ctype = ctype;
    g_p2c = p2c;
    g_c2p = c2p;
    a_rmdir(B "/t");
    pm_in_ns(scenario);
    a_rmdir(B "/t");
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    a_rmdir(B "/t");
    a_rmdir(B);
    pm_mkdirs(B);
    run("shared_shared", MS_SHARED, 0, 1, 1);
    run("shared_slave", MS_SHARED, MS_SLAVE, 1, 0);
    run("shared_private", MS_SHARED, MS_PRIVATE, 0, 0);
    run("never_shared", 0, 0, 0, 0);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
