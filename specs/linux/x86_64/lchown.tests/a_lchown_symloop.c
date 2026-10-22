/* Symbolic links followed during lookup (middle components, or a last
 * component with a trailing slash) count against a limit of 40 per
 * lookup: a chain of 40 links resolves, 41 is ELOOP. The last component
 * without a trailing slash is not followed, so lchown of the head of a
 * long chain acts on the head link. Links with absolute targets count
 * the same. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_lchown 94
#define D "/tmp/a_lchown_symloop"

static i64 lchown(const char *p, i64 uid, i64 gid) { return syscall3(SYS_lchown, p, uid, gid); }

static char name[128], target[128];

/* D/<prefix><i> */
static char *link_name(char *out, const char *prefix, int i) {
    char num[24];
    char *e = str_cpy(str_cpy(out, D "/"), prefix);
    str_cpy(e, a_utoa(num, i));
    return out;
}

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_dir", a_mkdir(D "/dir", 0755), 0);
    expect("setup_file", a_make_file(D "/dir/f", "", 0644), 0);
    /* c1 -> c2 -> ... -> c41 -> dir (relative); a1 -> ... -> a41 -> dir (absolute) */
    int ok = 1;
    for (int i = 1; i <= 41; i++) {
        link_name(name, "c", i);
        char num[24];
        if (i == 41)
            str_cpy(target, "dir");
        else
            str_cat2(target, "c", a_utoa(num, i + 1));
        ok &= syscall3(SYS_symlinkat, target, AT_FDCWD, name) == 0;
        link_name(name, "a", i);
        if (i == 41)
            str_cpy(target, D "/dir");
        else
            link_name(target, "a", i + 1);
        ok &= syscall3(SYS_symlinkat, target, AT_FDCWD, name) == 0;
    }
    check("setup_chains", ok, 0);

    /* From c2 there are 40 links to "dir", from c1 41. */
    expect("chain_40_slash", lchown(D "/c2/", uid, gid), 0);
    expect("chain_40_middle", lchown(D "/c2/f", uid, gid), 0);
    expect("chain_41_slash_eloop", lchown(D "/c1/", -1, -1), -ELOOP);
    expect("chain_41_middle_eloop", lchown(D "/c1/f", -1, -1), -ELOOP);
    expect("chain_41_missing_eloop", lchown(D "/c1/nope", -1, -1), -ELOOP);
    expect("chain_41_head_itself", lchown(D "/c1", uid, gid), 0);
    expect("abs_chain_40_middle", lchown(D "/a2/f", -1, -1), 0);
    expect("abs_chain_41_middle_eloop", lchown(D "/a1/f", -1, -1), -ELOOP);
    expect("abs_chain_41_head_itself", lchown(D "/a1", -1, -1), 0);
    /* The count is per lookup, not per link chain: from c22 there are 20
     * links to "dir", so c22/../c22/f follows 40 and c22/../c21/f 41. */
    expect("chain_20_then_20_ok", lchown(D "/c22/../c22/f", -1, -1), 0);
    expect("chain_20_then_21_eloop", lchown(D "/c22/../c21/f", -1, -1), -ELOOP);
    expect("chain_20_then_20_slash_ok", lchown(D "/c22/../c22/", -1, -1), 0);

    for (int i = 1; i <= 41; i++) {
        a_unlink(link_name(name, "c", i));
        a_unlink(link_name(name, "a", i));
    }
    expect("cleanup_f", a_unlink(D "/dir/f"), 0);
    expect("cleanup_dir", a_rmdir(D "/dir"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
