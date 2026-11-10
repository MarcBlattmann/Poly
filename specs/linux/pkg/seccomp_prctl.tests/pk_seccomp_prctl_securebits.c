/* The secure bits (PR_SET_SECUREBITS 28, PR_GET_SECUREBITS 27) beyond
 * SECBIT_NOROOT: which values exist, how the lock bits work, and what the
 * bits do to other calls.
 * Values (linux/securebits.h): six bits, each with a lock bit one position
 * higher: NOROOT 0x1 (lock 0x2), NO_SETUID_FIXUP 0x4 (0x8), KEEP_CAPS 0x10
 * (0x20), NO_CAP_AMBIENT_RAISE 0x40 (0x80), EXEC_RESTRICT_FILE 0x100
 * (0x200), EXEC_DENY_INTERACTIVE 0x400 (0x800). SET replaces the whole
 * value; any of these may be given, alone or together. A bit above 0x800
 * (0x1000, 0x10000, bit 32) is EPERM and changes nothing. arg3..arg5 are
 * ignored. Without CAP_SETPCAP a SET that changes any of the first four
 * pairs (0x1 .. 0x80) is EPERM, and so is a SET of the value that is
 * already there; the two EXEC pairs (0x100 .. 0x800) are the exception:
 * a caller without the capability may set and clear them (and lock them),
 * as long as nothing else changes in the same call.
 * Locks: while a lock bit is set, SET must keep that lock bit and must
 * keep its base bit as it is (set or clear): otherwise EPERM. Other bits
 * may change in the same call.
 * KEEP_CAPS is the same flag as PR_SET_KEEPCAPS (8) / PR_GET_KEEPCAPS (7):
 * each shows the other's change; PR_SET_KEEPCAPS takes 0 or 1 (2 is
 * EINVAL), needs no capability, and is EPERM while KEEP_CAPS is locked.
 * NO_CAP_AMBIENT_RAISE makes PR_CAP_AMBIENT_RAISE fail with EPERM; what is
 * already in the ambient set stays and can be lowered.
 * Leaving uid 0 with setresuid: normally the effective and permitted sets
 * are emptied; with NO_SETUID_FIXUP both stay; with KEEP_CAPS the
 * permitted set stays and the effective one is emptied (the bit itself
 * stays set).
 * poly-diff: needs=root
 * poly-diff: syscalls=off (every case forks)
 */
#include "p_seccomp.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setresuid 117
#define SYS_setresgid 119
#define PR_GET_KEEPCAPS 7
#define PR_SET_KEEPCAPS 8
#define PR_GET_SECUREBITS 27
#define PR_SET_SECUREBITS 28
#define PR_CAP_AMBIENT 47
#define AMB_IS_SET 1
#define AMB_RAISE 2
#define AMB_LOWER 3
#define CAP_CHOWN 0
#define CAP_KILL 5
#define CAP_V3 0x20080522

struct cap_header {
    u32 version;
    i32 pid;
};
struct cap_data {
    u32 effective, permitted, inheritable;
};

#define P(o, a, b, c, d) syscall5(SYS_prctl, (o), (a), (b), (c), (d))
#define GET() P(PR_GET_SECUREBITS, 0, 0, 0, 0)
#define SET(v) P(PR_SET_SECUREBITS, (v), 0, 0, 0)

static void values(void) {
    expect("start_0", GET(), 0);
    expect("no_setuid_fixup", SET(0x4), 0);
    expect("reads_0x4", GET(), 0x4);
    expect("replaced_by_keep_caps", SET(0x10), 0);
    expect("reads_0x10", GET(), 0x10);
    expect("no_cap_ambient_raise", SET(0x40), 0);
    expect("reads_0x40", GET(), 0x40);
    expect("exec_restrict_file", SET(0x100), 0);
    expect("reads_0x100", GET(), 0x100);
    expect("exec_deny_interactive", SET(0x400), 0);
    expect("reads_0x400", GET(), 0x400);
    expect("several_at_once", SET(0x554), 0);
    expect("reads_0x554", GET(), 0x554);
    expect("bit_0x1000_eperm", SET(0x1000), -EPERM);
    expect("bit_0x10000_eperm", SET(0x10000), -EPERM);
    expect("valid_plus_unknown_eperm", SET(0x1004), -EPERM);
    expect("bit_32_eperm", SET(0x100000000UL), -EPERM);
    expect("unchanged_by_refused_sets", GET(), 0x554);
    expect("other_arguments_ignored", P(PR_SET_SECUREBITS, 0x5, 7, 8, 9), 0);
    expect("reads_0x5", GET(), 0x5);
    expect("back_to_0", SET(0), 0);
    expect("reads_0", GET(), 0);
}

static void locks(void) {
    expect("fixup_and_lock", SET(0xc), 0);
    expect("reads_0xc", GET(), 0xc);
    expect("clear_all_eperm", SET(0), -EPERM);
    expect("clear_base_keep_lock_eperm", SET(0x8), -EPERM);
    expect("clear_lock_keep_base_eperm", SET(0x4), -EPERM);
    expect("still_0xc", GET(), 0xc);
    expect("add_unlocked_bit", SET(0x1c), 0);
    expect("reads_0x1c", GET(), 0x1c);
    expect("drop_unlocked_bit", SET(0xc), 0);
    expect("reads_0xc_again", GET(), 0xc);
}

static void lock_alone(void) {
    /* a lock without its base bit: the base bit can no longer be set */
    expect("lock_of_exec_deny", SET(0x800), 0);
    expect("reads_0x800", GET(), 0x800);
    expect("set_locked_base_eperm", SET(0xc00), -EPERM);
    expect("same_value_ok", SET(0x800), 0);
    expect("another_bit_ok", SET(0x900), 0);
    expect("reads_0x900", GET(), 0x900);
    expect("drop_lock_eperm", SET(0x100), -EPERM);
}

static void keep_caps(void) {
    expect("keepcaps_starts_0", P(PR_GET_KEEPCAPS, 0, 0, 0, 0), 0);
    expect("set_keep_caps_bit", SET(0x10), 0);
    expect("keepcaps_reads_1", P(PR_GET_KEEPCAPS, 0, 0, 0, 0), 1);
    expect("prctl_keepcaps_0", P(PR_SET_KEEPCAPS, 0, 0, 0, 0), 0);
    expect("bit_cleared", GET(), 0);
    expect("prctl_keepcaps_1", P(PR_SET_KEEPCAPS, 1, 0, 0, 0), 0);
    expect("bit_set", GET(), 0x10);
    expect("prctl_keepcaps_2_einval", P(PR_SET_KEEPCAPS, 2, 0, 0, 0), -EINVAL);
    expect("lock_keep_caps", SET(0x30), 0);
    expect("locked_keepcaps_0_eperm", P(PR_SET_KEEPCAPS, 0, 0, 0, 0), -EPERM);
    expect("locked_keepcaps_1_eperm", P(PR_SET_KEEPCAPS, 1, 0, 0, 0), -EPERM);
    expect("locked_value", GET(), 0x30);
}

static void ambient_raise(void) {
    struct cap_header h = {CAP_V3, 0};
    struct cap_data d[2];
    expect("capget", syscall2(SYS_capget, &h, d), 0);
    d[0].inheritable |= (1u << CAP_CHOWN) | (1u << CAP_KILL);
    expect("capset_inheritable", syscall2(SYS_capset, &h, d), 0);
    expect("raise_chown", P(PR_CAP_AMBIENT, AMB_RAISE, CAP_CHOWN, 0, 0), 0);
    expect("set_no_raise_bit", SET(0x40), 0);
    expect("raised_one_stays", P(PR_CAP_AMBIENT, AMB_IS_SET, CAP_CHOWN, 0, 0), 1);
    expect("raise_kill_eperm", P(PR_CAP_AMBIENT, AMB_RAISE, CAP_KILL, 0, 0), -EPERM);
    expect("raise_present_one_eperm", P(PR_CAP_AMBIENT, AMB_RAISE, CAP_CHOWN, 0, 0), -EPERM);
    expect("lower_still_works", P(PR_CAP_AMBIENT, AMB_LOWER, CAP_CHOWN, 0, 0), 0);
    expect("lowered", P(PR_CAP_AMBIENT, AMB_IS_SET, CAP_CHOWN, 0, 0), 0);
    expect("clear_no_raise_bit", SET(0), 0);
    expect("raise_kill_now", P(PR_CAP_AMBIENT, AMB_RAISE, CAP_KILL, 0, 0), 0);
}

static void unprivileged(void) {
    expect("root_sets_0x41", SET(0x41), 0);
    syscall3(SYS_setresgid, 65534, 65534, 65534);
    syscall3(SYS_setresuid, 65534, 65534, 65534);
    expect("user_reads_0x41", GET(), 0x41);
    expect("user_same_value_eperm", SET(0x41), -EPERM);
    expect("user_clear_eperm", SET(0), -EPERM);
    expect("user_clear_one_eperm", SET(0x40), -EPERM);
    expect("user_add_fixup_eperm", SET(0x45), -EPERM);
    expect("user_unknown_bit_eperm", SET(0x2041), -EPERM);
    expect("user_still_0x41", GET(), 0x41);
    /* the two EXEC pairs are the exception: no capability needed */
    expect("user_add_exec_restrict", SET(0x141), 0);
    expect("user_reads_0x141", GET(), 0x141);
    expect("user_same_value_again_eperm", SET(0x141), -EPERM);
    expect("user_exec_and_other_change_eperm", SET(0x540), -EPERM);
    expect("user_clear_exec_restrict", SET(0x41), 0);
    expect("user_exec_deny_with_lock", SET(0xc41), 0);
    expect("user_locked_exec_clear_eperm", SET(0x41), -EPERM);
    expect("user_reads_0xc41", GET(), 0xc41);
    expect("user_keepcaps_allowed", P(PR_SET_KEEPCAPS, 1, 0, 0, 0), 0);
    expect("user_reads_0xc51", GET(), 0xc51);
}

/* setresuid(65534) with the given secure bits: 2 * (permitted non-empty) + (effective non-empty) */
static i64 caps_after_setuid(u64 bits) {
    volatile i32 *s = a_shared();
    s[20] = -1;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        struct cap_header h = {CAP_V3, 0};
        struct cap_data d[2];
        if (SET(bits) != 0) syscall1(SYS_exit_group, 1);
        if (syscall3(SYS_setresuid, 65534, 65534, 65534) != 0) syscall1(SYS_exit_group, 2);
        if (syscall2(SYS_capget, &h, d) != 0) syscall1(SYS_exit_group, 3);
        s[20] = ((d[0].permitted | d[1].permitted) ? 2 : 0) + ((d[0].effective | d[1].effective) ? 1 : 0);
        s[21] = (i32)GET();
        syscall1(SYS_exit_group, 0);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    return st == 0 ? s[20] : -100 - st;
}

int test_main(void) {
    a_block_sigchld();
    volatile i32 *s = a_shared();
    pk_in_child(values);
    pk_in_child(locks);
    pk_in_child(lock_alone);
    pk_in_child(keep_caps);
    pk_in_child(ambient_raise);
    pk_in_child(unprivileged);
    expect("setuid_plain_empties_both", caps_after_setuid(0), 0);
    expect("setuid_no_fixup_keeps_both", caps_after_setuid(0x4), 3);
    expect("setuid_keep_caps_keeps_permitted", caps_after_setuid(0x10), 2);
    expect("keep_caps_bit_survives_setuid", s[21], 0x10);
    expect("parent_bits_untouched", GET(), 0);
    return finish();
}
