/* UTS namespaces keep the host name and the NIS domain name apart. A child
 * made with clone(CLONE_NEWUTS) starts with copies of its parent's names;
 * setting the names there (sethostname, setdomainname) changes what uname
 * and /proc/sys/kernel/{hostname,domainname} show in that namespace and
 * nowhere else; a plain fork child shares the namespace, so its change is
 * seen by its parent; a namespace made inside another one (unshare of
 * CLONE_NEWUTS) starts with the creator's current names and later changes
 * on either side stay on that side. Writing the new name to
 * /proc/sys/kernel/hostname has the same effect as sethostname. A name of
 * 64 bytes is accepted (not NUL-terminated, the length is the argument)
 * and reads back whole; 65 bytes is EINVAL for both calls; a length of 0
 * is accepted and makes the name empty; a NULL pointer with a non-zero
 * length is EFAULT. The other uname fields (sysname, release, version,
 * machine) are the same in every namespace.
 * poly-diff: needs=root (CAP_SYS_ADMIN in the user namespace that owns the UTS namespace)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

static char host0[65], dom0[65], sys0[65], rel0[65];

static void names(char *host, char *dom, char *sys, char *rel) {
    static char u[390];
    syscall1(SYS_uname, u);
    if (sys) str_cpy(sys, u);
    if (rel) str_cpy(rel, u + 130);
    if (host) str_cpy(host, u + 65);
    if (dom) str_cpy(dom, u + 325);
}

static int same(const char *a, const char *b) { return str_eq(a, b); }

static int inner(void) {
    char h[65], d[65];
    names(h, d, 0, 0);
    check("inner_starts_with_creators_host", same(h, "pk-a"), 0);
    check("inner_starts_with_creators_domain", same(d, "pk-a.dom"), 0);
    expect("inner_set_host", syscall2(SYS_sethostname, "pk-b", 4), 0);
    expect("inner_set_domain", syscall2(SYS_setdomainname, "pk-b.dom", 8), 0);
    names(h, d, 0, 0);
    check("inner_host_b", same(h, "pk-b"), 0);
    check("inner_domain_b", same(d, "pk-b.dom"), 0);
    return failed ? 1 : 0;
}

static int forked_child(void) {
    expect("fork_child_set", syscall2(SYS_sethostname, "pk-a2", 5), 0);
    return failed ? 1 : 0;
}

static int body(void) {
    char h[65], d[65], s[65], r[65], big[80], buf[256];
    names(h, d, s, r);
    check("copy_host", same(h, host0), 0);
    check("copy_domain", same(d, dom0), 0);
    expect("set_host", syscall2(SYS_sethostname, "pk-a", 4), 0);
    expect("set_domain", syscall2(SYS_setdomainname, "pk-a.dom", 8), 0);
    names(h, d, 0, 0);
    check("host_a", same(h, "pk-a"), 0);
    check("domain_a", same(d, "pk-a.dom"), 0);
    expect("proc_hostname", p_read_file("/proc/sys/kernel/hostname", buf, sizeof buf), 5);
    check("proc_hostname_text", same(buf, "pk-a\n"), 0);
    expect("proc_domainname", p_read_file("/proc/sys/kernel/domainname", buf, sizeof buf), 9);
    check("proc_domainname_text", same(buf, "pk-a.dom\n"), 0);

    p_run("inner_ns", CLONE_NEWUTS, inner);
    names(h, d, 0, 0);
    check("outer_host_unchanged", same(h, "pk-a"), 0);
    check("outer_domain_unchanged", same(d, "pk-a.dom"), 0);

    p_run("forked_child", 0, forked_child);
    names(h, 0, 0, 0);
    check("fork_child_change_seen", same(h, "pk-a2"), 0);

    expect("proc_write", p_write_file("/proc/sys/kernel/hostname", "pk-w"), 4);
    names(h, 0, 0, 0);
    check("proc_write_effect", same(h, "pk-w"), 0);

    for (int i = 0; i < 64; i++) big[i] = 'a' + (i % 26);
    big[64] = 'z';
    expect("set_host_64", syscall2(SYS_sethostname, big, 64), 0);
    names(h, 0, 0, 0);
    expect("host_64_len", (i64)str_len(h), 64);
    check("host_64_text", h[0] == 'a' && h[63] == 'a' + (63 % 26), 0);
    expect("set_host_65", syscall2(SYS_sethostname, big, 65), -EINVAL);
    expect("set_domain_64", syscall2(SYS_setdomainname, big, 64), 0);
    expect("set_domain_65", syscall2(SYS_setdomainname, big, 65), -EINVAL);
    expect("set_host_0", syscall2(SYS_sethostname, "x", 0), 0);
    names(h, 0, 0, 0);
    expect("host_empty", (i64)str_len(h), 0);
    expect("set_host_null", syscall2(SYS_sethostname, 0, 3), -EFAULT);

    char s2[65], r2[65];
    names(0, 0, s2, r2);
    check("sysname_same", same(s, s2) && same(s, "Linux"), 0);
    check("release_same", same(r, r2) && same(r, rel0), 0);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    names(host0, dom0, sys0, rel0);
    p_run("utsns", CLONE_NEWUTS, body);
    char h[65], d[65];
    names(h, d, 0, 0);
    check("parent_host_unchanged", same(h, host0), 0);
    check("parent_domain_unchanged", same(d, dom0), 0);
    return finish();
}
