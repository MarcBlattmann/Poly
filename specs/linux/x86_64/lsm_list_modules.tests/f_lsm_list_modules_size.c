/* lsm_list_modules(2) size protocol: when *size is smaller than the list
 * (number of modules * 8) the call fails with E2BIG, writes nothing to ids
 * and sets *size to the bytes needed; ids may then be NULL. A size that is
 * not a multiple of 8 but large enough works.
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

int test_main(void) {
    u64 ids[32];
    unsigned size = sizeof ids;
    i64 n = syscall3(SYS_lsm_list_modules, ids, &size, 0);
    check("listed", n >= 1, n);
    unsigned need = (unsigned)n * 8;
    unsigned sz = 0;
    i64 r = syscall3(SYS_lsm_list_modules, 0, &sz, 0);
    check("null_ids_size0_e2big", r == -E2BIG, r);
    check("size0_set_to_need", sz == need, sz);
    memset(ids, 0xee, sizeof ids);
    sz = 0;
    r = syscall3(SYS_lsm_list_modules, ids, &sz, 0);
    check("size0_e2big", r == -E2BIG && sz == need, r);
    check("size0_untouched", ids[0] == 0xeeeeeeeeeeeeeeeeUL, 0);
    sz = need - 1;
    r = syscall3(SYS_lsm_list_modules, ids, &sz, 0);
    check("one_short_e2big", r == -E2BIG && sz == need, r);
    check("one_short_untouched", ids[0] == 0xeeeeeeeeeeeeeeeeUL, 0);
    sz = 8;
    r = syscall3(SYS_lsm_list_modules, ids, &sz, 0);
    check("size8", (n == 1 && r == 1 && sz == 8) || (n > 1 && r == -E2BIG && sz == need), r);
    sz = need + 3;
    r = syscall3(SYS_lsm_list_modules, ids, &sz, 0);
    check("odd_large_size_ok", r == n && sz == need, r == n);
    return finish();
}
