/* lsm_set_self_attr(2) for modules that provide no attributes: with a
 * well-formed struct lsm_ctx whose id is capability (100), landlock (110),
 * LSM_ID_UNDEF (0) or no module (999), every attribute value (0, 1,
 * CURRENT, EXEC, FSCREATE, KEYCREATE, PREV, SOCKCREATE, 999) is
 * EOPNOTSUPP; ctx->flags is not looked at. The caller's attributes are
 * unchanged afterwards (lsm_get_self_attr gives the same answer).
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_add_key 248
#define SYS_request_key 249
#define SYS_keyctl 250
#define SYS_lsm_get_self_attr 459
#define SYS_lsm_set_self_attr 460
#define SYS_lsm_list_modules 461
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define E2BIG 7
#define EDQUOT 122
#define ENOKEY 126
#define EKEYEXPIRED 127
#define EKEYREVOKED 128
#define EKEYREJECTED 129
/* linux/keyctl.h: special keyring ids */
#define KEY_SPEC_THREAD_KEYRING (-1)
#define KEY_SPEC_PROCESS_KEYRING (-2)
#define KEY_SPEC_SESSION_KEYRING (-3)
#define KEY_SPEC_USER_KEYRING (-4)
#define KEY_SPEC_USER_SESSION_KEYRING (-5)
#define KEY_SPEC_GROUP_KEYRING (-6)
#define KEY_SPEC_REQKEY_AUTH_KEY (-7)
#define KEY_SPEC_REQUESTOR_KEYRING (-8)
/* linux/keyctl.h: request-key default keyrings */
#define KEY_REQKEY_DEFL_NO_CHANGE (-1)
#define KEY_REQKEY_DEFL_DEFAULT 0
#define KEY_REQKEY_DEFL_SESSION_KEYRING 3
/* linux/keyctl.h: operations */
#define KEYCTL_GET_KEYRING_ID 0
#define KEYCTL_JOIN_SESSION_KEYRING 1
#define KEYCTL_UPDATE 2
#define KEYCTL_REVOKE 3
#define KEYCTL_CHOWN 4
#define KEYCTL_SETPERM 5
#define KEYCTL_DESCRIBE 6
#define KEYCTL_CLEAR 7
#define KEYCTL_LINK 8
#define KEYCTL_UNLINK 9
#define KEYCTL_SEARCH 10
#define KEYCTL_READ 11
#define KEYCTL_INSTANTIATE 12
#define KEYCTL_NEGATE 13
#define KEYCTL_SET_REQKEY_KEYRING 14
#define KEYCTL_SET_TIMEOUT 15
#define KEYCTL_ASSUME_AUTHORITY 16
#define KEYCTL_GET_SECURITY 17
#define KEYCTL_SESSION_TO_PARENT 18
#define KEYCTL_REJECT 19
#define KEYCTL_INSTANTIATE_IOV 20
#define KEYCTL_INVALIDATE 21
#define KEYCTL_GET_PERSISTENT 22
#define KEYCTL_MOVE 30
#define KEYCTL_MOVE_EXCL 0x00000001
/* linux/key.h (permission bits, also in keyutils.h) */
#define KEY_POS_VIEW 0x01000000
#define KEY_POS_READ 0x02000000
#define KEY_POS_WRITE 0x04000000
#define KEY_POS_SEARCH 0x08000000
#define KEY_POS_LINK 0x10000000
#define KEY_POS_SETATTR 0x20000000
#define KEY_POS_ALL 0x3f000000
#define KEY_USR_VIEW 0x00010000
/* linux/lsm.h */
#define LSM_ID_UNDEF 0
#define LSM_ID_CAPABILITY 100
#define LSM_ID_SELINUX 101
#define LSM_ID_LANDLOCK 110
#define LSM_ID_IPE 113
#define LSM_ATTR_UNDEF 0
#define LSM_ATTR_CURRENT 100
#define LSM_ATTR_EXEC 101
#define LSM_ATTR_FSCREATE 102
#define LSM_ATTR_KEYCREATE 103
#define LSM_ATTR_PREV 104
#define LSM_ATTR_SOCKCREATE 105
#define LSM_FLAG_SINGLE 0x0001
/* struct lsm_ctx (linux/lsm.h): four u64, then ctx[ctx_len], len padded. */
struct k_lsm_ctx {
    u64 id, flags, len, ctx_len;
    unsigned char ctx[];
};
#define LSM_CTX_HDR 32

static __attribute__((unused)) i64 add_key(const char *type, const char *desc, const void *p, u64 n, i64 ring) {
    return syscall5(SYS_add_key, type, desc, p, n, ring);
}
static __attribute__((unused)) i64 request_key(const char *type, const char *desc, const char *info, i64 ring) {
    return syscall4(SYS_request_key, type, desc, info, ring);
}
#define keyctl2(op, a) syscall2(SYS_keyctl, (op), (a))
#define keyctl3(op, a, b) syscall3(SYS_keyctl, (op), (a), (b))
#define keyctl4(op, a, b, c) syscall4(SYS_keyctl, (op), (a), (b), (c))
#define keyctl5(op, a, b, c, d) syscall5(SYS_keyctl, (op), (a), (b), (c), (d))

/* Every keys test starts in an anonymous session keyring of its own, so
 * nothing depends on keyrings that exist before it runs. */
static __attribute__((unused)) i64 fresh_session(void) { return keyctl2(KEYCTL_JOIN_SESSION_KEYRING, 0); }

/* Decimal and 8-digit lower-case hex, for the expected DESCRIBE text. */
static __attribute__((unused)) char *fmt_u(char *p, u64 v) {
    char t[24];
    int n = 0;
    do t[n++] = (char)('0' + v % 10);
    while (v /= 10);
    while (n) *p++ = t[--n];
    return p;
}
static __attribute__((unused)) char *fmt_x8(char *p, u64 v) {
    for (int i = 7; i >= 0; i--) *p++ = "0123456789abcdef"[(v >> (i * 4)) & 15];
    return p;
}
static __attribute__((unused)) char *fmt_s(char *p, const char *s) {
    while (*s) *p++ = *s++;
    return p;
}
/* "type;uid;gid;perm;description" with the caller's uid and gid. */
static __attribute__((unused)) void expect_desc(char *out, const char *type, u64 perm, const char *desc) {
    char *p = fmt_s(out, type);
    *p++ = ';';
    p = fmt_u(p, (u64)syscall0(SYS_getuid));
    *p++ = ';';
    p = fmt_u(p, (u64)syscall0(SYS_getgid));
    *p++ = ';';
    p = fmt_x8(p, perm);
    *p++ = ';';
    p = fmt_s(p, desc);
    *p = 0;
}
/* DESCRIBE a key and compare with the expected text (length includes NUL). */
static __attribute__((unused)) int describe_is(i64 key, const char *type, u64 perm, const char *desc) {
    char want[256], got[256];
    expect_desc(want, type, perm, desc);
    memset(got, 0, sizeof got);
    i64 r = keyctl4(KEYCTL_DESCRIBE, key, got, sizeof got);
    return r == (i64)str_len(want) + 1 && str_eq(got, want);
}
static __attribute__((unused)) void *map_anon(u64 len, int prot) {
    return (void *)syscall6(SYS_mmap, 0, (i64)len, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}
/* A child the parent waits for before it continues (clone with
 * CLONE_VFORK, own copy of memory); SIGCHLD blocked so nothing interrupts. */
static __attribute__((unused)) i64 fork_sync_k(void) {
    sig_mask(SIG_BLOCK, SIGBIT(SIGCHLD), 0);
    return syscall5(SYS_clone, 0x4000 /* CLONE_VFORK */ | SIGCHLD, 0, 0, 0, 0);
}

static u64 page[64];
static u64 before[256], after[256];

int test_main(void) {
    unsigned szb = sizeof before;
    i64 nb = syscall4(SYS_lsm_get_self_attr, LSM_ATTR_CURRENT, before, &szb, 0);
    static const u64 ids[] = { LSM_ID_CAPABILITY, LSM_ID_LANDLOCK, LSM_ID_UNDEF, 999 };
    static const u64 attrs[] = { 0, 1, 100, 101, 102, 103, 104, 105, 999 };
    struct k_lsm_ctx *c = (struct k_lsm_ctx *)page;
    int all = 1;
    i64 bad = 0;
    for (unsigned i = 0; i < 4; i++) {
        for (unsigned a = 0; a < 9; a++) {
            memset(page, 0, sizeof page);
            c->id = ids[i];
            c->len = 40;
            c->ctx_len = 7;
            memcpy(c->ctx, "kernel", 7);
            i64 r = syscall4(SYS_lsm_set_self_attr, attrs[a], c, 40, 0);
            if (r != -EOPNOTSUPP) {
                all = 0;
                bad = (i64)(ids[i] * 1000 + attrs[a]);
            }
        }
    }
    check("all_eopnotsupp", all, bad);
    memset(page, 0, sizeof page);
    c->id = LSM_ID_CAPABILITY;
    c->flags = 1;
    c->len = 40;
    c->ctx_len = 7;
    i64 r = syscall4(SYS_lsm_set_self_attr, LSM_ATTR_CURRENT, c, 40, 0);
    check("ctx_flags_ignored_eopnotsupp", r == -EOPNOTSUPP, r);
    unsigned sza = sizeof after;
    i64 na = syscall4(SYS_lsm_get_self_attr, LSM_ATTR_CURRENT, after, &sza, 0);
    check("attributes_unchanged", na == nb && sza == szb && (nb < 0 || mem_eq(before, after, szb)), na == nb);
    return finish();
}
