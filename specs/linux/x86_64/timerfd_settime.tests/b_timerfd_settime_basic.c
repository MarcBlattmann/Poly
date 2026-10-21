/* timerfd_settime: a relative one-shot timer expires after its value (a
 * blocking read waits for it and returns the expiration count, 1); the
 * old setting is returned; a periodic timer counts every interval that
 * has passed since the last read; TFD_TIMER_ABSTIME takes a deadline on
 * the timer's clock (one in the past expires at once); a zero value
 * disarms the timer and discards a pending count. Errors: tv_nsec out of
 * range or negative values (EINVAL), unknown flags (EINVAL), a non-timerfd
 * descriptor (EINVAL), a bad pointer (EFAULT). */
#include "poly_ipc.h"

int test_main(void) {
    int fd = (int)syscall2(SYS_timerfd_create, CLOCK_MONOTONIC, 0);
    struct k_itimerspec one = { { 0, 0 }, { 0, 30000000 } }, old, off;
    memset(&off, 0, sizeof off);
    i64 t0 = now_ns();
    i64 r = syscall4(SYS_timerfd_settime, fd, 0, &one, &old);
    check("settime_ok", r == 0, r);
    check("old_was_disarmed", old.value.tv_sec == 0 && old.value.tv_nsec == 0, old.value.tv_nsec);
    u64 n = 0;
    r = read(fd, &n, 8);
    i64 dt = now_ns() - t0;
    check("read_waits_for_expiry", r == 8 && n == 1, (i64)n);
    check("expired_after_value", dt >= 29000000, dt / 1000000);
    struct k_itimerspec cur;
    syscall2(SYS_timerfd_gettime, fd, &cur);
    check("one_shot_disarmed_after", cur.value.tv_sec == 0 && cur.value.tv_nsec == 0, cur.value.tv_nsec);

    /* Periodic: 10 ms; after ~55 ms at least 4 expirations are counted. */
    struct k_itimerspec per = { { 0, 10000000 }, { 0, 10000000 } };
    syscall4(SYS_timerfd_settime, fd, 0, &per, 0);
    sleep_ms(55);
    read(fd, &n, 8);
    check("periodic_counts_expirations", n >= 4 && n <= 50, (i64)n);
    r = syscall4(SYS_timerfd_settime, fd, 0, &off, &old);
    check("disarm_ok", r == 0, r);
    check("old_interval_returned", old.interval.tv_sec == 0 && old.interval.tv_nsec == 10000000,
          old.interval.tv_nsec);
    check("old_value_within_interval", old.value.tv_sec == 0 && old.value.tv_nsec > 0 && old.value.tv_nsec <= 10000000,
          old.value.tv_nsec);

    /* Expired but unread, then disarmed: the count is discarded. */
    int nb = (int)syscall2(SYS_timerfd_create, CLOCK_MONOTONIC, TFD_NONBLOCK);
    struct k_itimerspec soon = { { 0, 0 }, { 0, 1000000 } };
    syscall4(SYS_timerfd_settime, nb, 0, &soon, 0);
    sleep_ms(10);
    struct k_pollfd pf = { nb, POLLIN, 0 };
    r = syscall3(SYS_poll, &pf, 1, 0);
    check("expired_readable", r == 1 && pf.revents == POLLIN, pf.revents);
    syscall4(SYS_timerfd_settime, nb, 0, &off, 0);
    r = read(nb, &n, 8);
    check("disarm_discards_count", r == -EAGAIN, r);

    /* Absolute deadline in the past: expires at once. */
    struct k_timespec now;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &now);
    struct k_itimerspec past = { { 0, 0 }, { now.tv_sec - 1, now.tv_nsec } };
    r = syscall4(SYS_timerfd_settime, nb, TFD_TIMER_ABSTIME, &past, 0);
    check("abstime_ok", r == 0, r);
    r = read(nb, &n, 8);
    check("past_deadline_expired", r == 8 && n == 1, r);
    struct k_itimerspec ahead = { { 0, 0 }, { now.tv_sec + 100, 0 } };
    syscall4(SYS_timerfd_settime, nb, TFD_TIMER_ABSTIME, &ahead, 0);
    syscall2(SYS_timerfd_gettime, nb, &cur);
    check("abstime_gettime_relative", cur.value.tv_sec >= 98 && cur.value.tv_sec <= 100, cur.value.tv_sec);

    struct k_itimerspec bad = { { 0, 0 }, { 0, 1000000000 } };
    r = syscall4(SYS_timerfd_settime, nb, 0, &bad, 0);
    check("nsec_too_big_einval", r == -EINVAL, r);
    struct k_itimerspec badi = { { 0, -1 }, { 1, 0 } };
    r = syscall4(SYS_timerfd_settime, nb, 0, &badi, 0);
    check("negative_interval_nsec_einval", r == -EINVAL, r);
    struct k_itimerspec neg = { { 0, 0 }, { -1, 0 } };
    r = syscall4(SYS_timerfd_settime, nb, 0, &neg, 0);
    check("negative_value_einval", r == -EINVAL, r);
    r = syscall4(SYS_timerfd_settime, nb, 4, &one, 0);
    check("unknown_flag_einval", r == -EINVAL, r);
    r = syscall4(SYS_timerfd_settime, nb, 0, 8, 0);
    check("bad_new_pointer_efault", r == -EFAULT, r);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    r = syscall4(SYS_timerfd_settime, p[0], 0, &one, 0);
    check("not_a_timerfd_einval", r == -EINVAL, r);
    r = syscall4(SYS_timerfd_settime, 90, 0, &one, 0);
    check("closed_fd_ebadf", r == -EBADF, r);
    return finish();
}
