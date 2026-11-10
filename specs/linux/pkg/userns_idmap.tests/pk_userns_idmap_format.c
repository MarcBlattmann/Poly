/* The files that describe and set a user namespace's id mappings:
 * /proc/self/uid_map, gid_map and setgroups.
 *
 * The initial namespace maps everything to itself: reading uid_map gives
 * exactly "         0          0 4294967295\n" (three numbers, each in a
 * field ten characters wide, separated by one blank), setgroups "allow\n".
 * In a namespace made with unshare(CLONE_NEWUSER) the maps read empty
 * until written, setgroups reads "allow\n". A write of "0 0 1" (with or
 * without the final newline) is accepted and reads back in the ten-wide
 * format; a second write is EPERM. A process inside the namespace cannot
 * write gid_map before it wrote "deny" to setgroups (EPERM). setgroups
 * takes "allow" or "deny" (newline optional, anything else EINVAL); "deny"
 * reads back "deny\n"; "allow" after "deny" is EPERM; "deny" is EPERM once
 * gid_map was written (by the parent, which needs no "deny"). The parent
 * can write several lines in one write; they read back as several lines.
 * poly-diff: needs=root (a new user namespace mapped for a parent id other than the caller's)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

static char buf[4096];

static int in_ns_text(void) {
    expect("unshare_newuser", syscall1(SYS_unshare, CLONE_NEWUSER), 0);
    expect("uid_map_empty", p_read_file("/proc/self/uid_map", buf, sizeof buf), 0);
    expect("gid_map_empty", p_read_file("/proc/self/gid_map", buf, sizeof buf), 0);
    p_read_file("/proc/self/setgroups", buf, sizeof buf);
    check("setgroups_allow", str_eq(buf, "allow\n"), 0);

    expect("setgroups_garbage", p_write_file("/proc/self/setgroups", "maybe"), -EINVAL);
    expect("setgroups_upper", p_write_file("/proc/self/setgroups", "ALLOW"), -EINVAL);
    expect("setgroups_allow_ok", p_write_file("/proc/self/setgroups", "allow"), 5);

    expect("uid_map_no_newline", p_write_file("/proc/self/uid_map", "0 0 1"), 5);
    p_read_file("/proc/self/uid_map", buf, sizeof buf);
    check("uid_map_text", str_eq(buf, "         0          0          1\n"), 0);

    expect("setgroups_deny", p_write_file("/proc/self/setgroups", "deny\n"), 5);
    p_read_file("/proc/self/setgroups", buf, sizeof buf);
    check("setgroups_reads_deny", str_eq(buf, "deny\n"), 0);
    expect("setgroups_deny_again", p_write_file("/proc/self/setgroups", "deny"), 4);
    expect("setgroups_allow_after_deny", p_write_file("/proc/self/setgroups", "allow"), -EPERM);
    p_read_file("/proc/self/setgroups", buf, sizeof buf);
    check("setgroups_still_deny", str_eq(buf, "deny\n"), 0);

    expect("gid_map_written", p_write_file("/proc/self/gid_map", "0 0 1\n"), 6);
    p_read_file("/proc/self/gid_map", buf, sizeof buf);
    check("gid_map_text", str_eq(buf, "         0          0          1\n"), 0);
    expect("uid_map_second_write", p_write_file("/proc/self/uid_map", "0 0 1\n"), -EPERM);
    p_read_file("/proc/self/uid_map", buf, sizeof buf);
    check("uid_map_unchanged", str_eq(buf, "         0          0          1\n"), 0);
    return failed ? 1 : 0;
}

static int self_gid_map_needs_deny(void) {
    syscall1(SYS_unshare, CLONE_NEWUSER);
    expect("gid_map_without_deny", p_write_file("/proc/self/gid_map", "0 0 1\n"), -EPERM);
    expect("gid_map_still_empty", p_read_file("/proc/self/gid_map", buf, sizeof buf), 0);
    return failed ? 1 : 0;
}

/* The parent (root in the namespace the child came from) writes the maps. */
static void write_gid_map_only(i64 pid) {
    char path[48];
    expect("parent_gid_map", p_write_file(p_proc_path(path, pid, "gid_map"), "0 0 1\n"), 6);
}

static int in_ns_after_gid_map(void) {
    p_read_file("/proc/self/setgroups", buf, sizeof buf);
    check("setgroups_allow_kept", str_eq(buf, "allow\n"), 0);
    expect("deny_after_gid_map", p_write_file("/proc/self/setgroups", "deny"), -EPERM);
    return failed ? 1 : 0;
}

static void write_three_lines(i64 pid) {
    char path[48];
    expect("parent_three_lines",
           p_write_file(p_proc_path(path, pid, "uid_map"), "0 1000 1\n5 2000 3\n100 4000 10\n"), 30);
}

static int in_ns_lines(void) {
    p_read_file("/proc/self/uid_map", buf, sizeof buf);
    check("three_lines_text", str_eq(buf, "         0       1000          1\n"
                                          "         5       2000          3\n"
                                          "       100       4000         10\n"),
          0);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    p_read_file("/proc/self/uid_map", buf, sizeof buf);
    check("initial_uid_map", str_eq(buf, "         0          0 4294967295\n"), 0);
    p_read_file("/proc/self/gid_map", buf, sizeof buf);
    check("initial_gid_map", str_eq(buf, "         0          0 4294967295\n"), 0);
    p_read_file("/proc/self/setgroups", buf, sizeof buf);
    check("initial_setgroups", str_eq(buf, "allow\n"), 0);
    p_run("text_child_exit_0", 0, in_ns_text);
    p_run("self_gid_map_child_exit_0", 0, self_gid_map_needs_deny);
    p_run_with("after_gid_map_child_exit_0", CLONE_NEWUSER, in_ns_after_gid_map, write_gid_map_only);
    p_run_with("lines_child_exit_0", CLONE_NEWUSER, in_ns_lines, write_three_lines);
    return finish();
}
