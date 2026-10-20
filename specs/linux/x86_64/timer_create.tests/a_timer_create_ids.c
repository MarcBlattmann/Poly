/* timer_create: returns 0 and stores a timer id; ids start from 0 in a
 * fresh process and are handed out in increasing order (a deleted id is
 * not given out again at once); a NULL sigevent means SIGEV_SIGNAL with
 * SIGALRM. All clocks that can time a timer are accepted. */
#include "a_sys.h"

int test_main(void) {
    i32 ids[4];
    const int clocks[] = {CLOCK_REALTIME, CLOCK_MONOTONIC, CLOCK_BOOTTIME, CLOCK_PROCESS_CPUTIME_ID};
    struct a_sigevent ev;
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_NONE;
    int ok = 1;
    for (int i = 0; i < 4; i++) ok &= syscall3(SYS_timer_create, clocks[i], &ev, &ids[i]) == 0;
    check("create_on_four_clocks", ok, 0);
    check("ids_0_to_3", ids[0] == 0 && ids[1] == 1 && ids[2] == 2 && ids[3] == 3, ids[0]);
    i32 dflt = -1;
    expect("null_sigevent", syscall3(SYS_timer_create, CLOCK_MONOTONIC, 0, &dflt), 0);
    expect("next_id_4", dflt, 4);
    expect("thread_cputime_clock", syscall3(SYS_timer_create, CLOCK_THREAD_CPUTIME_ID, &ev, &ids[0]), 0);
    expect("its_id_5", ids[0], 5);
    for (int i = 0; i < 6; i++) syscall1(SYS_timer_delete, i);
    expect("create_after_deletes", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &ids[0]), 0);
    expect("next_id_not_reused", ids[0], 6);
    syscall1(SYS_timer_delete, ids[0]);
    return finish();
}
