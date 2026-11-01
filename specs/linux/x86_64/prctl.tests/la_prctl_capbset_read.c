/* PR_CAPBSET_READ (23, cap) returns 1 if capability cap is in the
 * calling thread's bounding set, 0 if not; cap must be a valid
 * capability, 0..CAP_LAST_CAP (40, CAP_CHECKPOINT_RESTORE, on Linux
 * 6.18): 41, 63, -1 and 2^32 are EINVAL (cap is a full 64-bit value).
 * Here every valid number answers 0 or 1, and a new program normally
 * starts with the full bounding set.
 */
#include "a_sys.h"

#define SYS_prctl 157
#define PR_CAPBSET_READ 23

static i64 rd(u64 cap) { return syscall5(SYS_prctl, PR_CAPBSET_READ, cap, 0, 0, 0); }

int test_main(void) {
    int bad = 0;
    for (u64 cap = 0; cap <= 40; cap++) {
        i64 r = rd(cap);
        if (r != 0 && r != 1) bad++;
    }
    expect("caps_0_to_40_answer_0_or_1", bad, 0);
    expect("cap_40_valid", rd(40) >= 0, 1);
    expect("cap_41_einval", rd(41), -EINVAL);
    expect("cap_63_einval", rd(63), -EINVAL);
    expect("cap_minus1_einval", rd((u64)-1), -EINVAL);
    expect("cap_2_32_einval", rd(1UL << 32), -EINVAL);
    return finish();
}
