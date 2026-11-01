/* ioprio_set(IOPRIO_WHO_PROCESS, 0, prio) sets the calling thread's I/O
 * priority and ioprio_get(IOPRIO_WHO_PROCESS, 0) reads it back unchanged.
 * prio is class << 13 | hint << 3 | level: the best-effort (2) and idle (3)
 * classes take levels 0..7 and are allowed to any user; class none (0)
 * only with level 0 (reads back as 0). The value is kept as 16 bits: the
 * hint bits 3..12 are kept as given, bits 16..31 are dropped, and the
 * upper half of the 64-bit register is ignored. Classes 4..7 and level
 * bits on class none are EINVAL and change nothing.
 */
#include "a_sys.h"

#define SYS_ioprio_set 251
#define SYS_ioprio_get 252
/* linux/ioprio.h */
#define IOPRIO_WHO_PROCESS 1
#define IOPRIO_CLASS_NONE 0
#define IOPRIO_CLASS_BE 2
#define IOPRIO_CLASS_IDLE 3
#define PRIO(c, l) (((c) << 13) | (l))

static i64 set(i64 prio) { return syscall3(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, prio); }
static i64 get(void) { return syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0); }

int test_main(void) {
    int ok = 1;
    for (int l = 0; l < 8; l++) {
        if (set(PRIO(IOPRIO_CLASS_BE, l)) != 0 || get() != PRIO(IOPRIO_CLASS_BE, l)) ok = 0;
    }
    check("be_levels_0_to_7", ok, get());
    ok = 1;
    for (int l = 0; l < 8; l++) {
        if (set(PRIO(IOPRIO_CLASS_IDLE, l)) != 0 || get() != PRIO(IOPRIO_CLASS_IDLE, l)) ok = 0;
    }
    check("idle_levels_0_to_7", ok, get());

    expect("set_be3", set(PRIO(IOPRIO_CLASS_BE, 3)), 0);
    expect("get_be3", get(), 0x4003);
    expect("set_none0", set(PRIO(IOPRIO_CLASS_NONE, 0)), 0);
    expect("get_none_is_0", get(), 0);

    expect("set_be5", set(PRIO(IOPRIO_CLASS_BE, 5)), 0);
    expect("none_level1_einval", set(PRIO(IOPRIO_CLASS_NONE, 1)), -EINVAL);
    expect("none_level7_einval", set(PRIO(IOPRIO_CLASS_NONE, 7)), -EINVAL);
    expect("class4_einval", set(PRIO(4, 0)), -EINVAL);
    expect("class5_einval", set(PRIO(5, 2)), -EINVAL);
    expect("class7_einval", set(PRIO(7, 0)), -EINVAL);
    expect("minus1_einval", set(-1), -EINVAL);
    expect("unchanged_after_einval", get(), 0x4005);

    /* Hint bits (3..12) are kept. */
    expect("set_be0_hint1", set(PRIO(IOPRIO_CLASS_BE, 0) | (1 << 3)), 0);
    expect("get_be0_hint1", get(), 0x4008);
    expect("set_be1_hint6", set(PRIO(IOPRIO_CLASS_BE, 1) | (6 << 3)), 0);
    expect("get_be1_hint6", get(), 0x4031);
    expect("set_idle2_hint3", set(PRIO(IOPRIO_CLASS_IDLE, 2) | (3 << 3)), 0);
    expect("get_idle2_hint3", get(), 0x601a);

    /* Bits above 15 are dropped; the register's upper half is ignored. */
    expect("set_bit16", set(PRIO(IOPRIO_CLASS_BE, 1) | (1 << 16)), 0);
    expect("get_bit16_dropped", get(), 0x4001);
    expect("set_upper_half", set((1L << 32) | PRIO(IOPRIO_CLASS_BE, 2)), 0);
    expect("get_upper_half_ignored", get(), 0x4002);
    expect("which_upper_half_ignored", syscall2(SYS_ioprio_get, (1L << 32) | IOPRIO_WHO_PROCESS, 0), 0x4002);
    return finish();
}
