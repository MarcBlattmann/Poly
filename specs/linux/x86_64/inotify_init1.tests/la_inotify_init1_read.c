/* Reading an inotify descriptor opened with IN_NONBLOCK: an empty queue
 * is EAGAIN (for any buffer size, also 0); FIONREAD gives the number of
 * queued bytes (0 when empty); poll reports POLLIN only while events are
 * queued. Each event is a 16-byte header {int wd; u32 mask; u32 cookie;
 * u32 len} followed by len bytes of name, NUL padded so that header plus
 * name is a multiple of 16 (a 1..15 byte name gives len 16). A read
 * returns as many whole events as fit; a buffer too small for the next
 * event is EINVAL (the event stays queued); a buffer of exactly one
 * event takes one.
 */
#include "a_sys.h"

#define SYS_ioctl 16
#define SYS_poll 7
#define SYS_inotify_add_watch 254
#define SYS_inotify_init1 294
/* asm-generic/ioctls.h */
#define FIONREAD 0x541B
/* asm-generic/poll.h */
#define POLLIN 0x1
/* linux/inotify.h */
#define IN_NONBLOCK O_NONBLOCK
#define IN_CREATE 0x100
#define IN_DELETE 0x200

struct k_inotify_event {
    i32 wd;
    u32 mask;
    u32 cookie;
    u32 len;
};
/* asm-generic/poll.h */
struct k_pollfd {
    i32 fd;
    short events, revents;
};

static i64 queued(i64 fd) {
    i32 n = -1;
    i64 r = syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return r < 0 ? r : n;
}
static i64 poll_in(i64 fd) {
    struct k_pollfd p = { (i32)fd, POLLIN, 0 };
    i64 r = syscall3(SYS_poll, &p, 1, 0);
    return r <= 0 ? r : p.revents;
}

static char buf[1024] __attribute__((aligned(8)));

#define DIR "/tmp/la_inotify_init1_read"

int test_main(void) {
    a_rmdir(DIR "/abc");
    a_unlink(DIR "/abc");
    a_unlink(DIR "/0123456789abcdef");
    a_rmdir(DIR);
    a_mkdir(DIR, 0755);
    i64 fd = syscall1(SYS_inotify_init1, IN_NONBLOCK);
    expect("empty_read_eagain", a_read(fd, buf, sizeof buf), -EAGAIN);
    expect("empty_read_0_eagain", a_read(fd, buf, 0), -EAGAIN);
    expect("empty_fionread_0", queued(fd), 0);
    expect("empty_poll_0", poll_in(fd), 0);

    i64 wd = syscall3(SYS_inotify_add_watch, fd, DIR, IN_CREATE | IN_DELETE);
    expect("watch_wd_1", wd, 1);
    a_make_file(DIR "/abc", "", 0644);
    a_make_file(DIR "/0123456789abcdef", "", 0644);
    a_unlink(DIR "/abc");

    /* 32 + 48 + 32 bytes queued. */
    expect("fionread_112", queued(fd), 112);
    expect("poll_pollin", poll_in(fd), POLLIN);
    expect("read_0_einval", a_read(fd, buf, 0), -EINVAL);
    expect("read_16_einval", a_read(fd, buf, 16), -EINVAL);
    expect("read_31_einval", a_read(fd, buf, 31), -EINVAL);
    expect("still_queued", queued(fd), 112);

    memset(buf, 0x55, sizeof buf);
    expect("read_exactly_one", a_read(fd, buf, 32), 32);
    struct k_inotify_event *e = (struct k_inotify_event *)buf;
    expect("ev1_wd", e->wd, 1);
    expect("ev1_mask_create", e->mask, IN_CREATE);
    expect("ev1_cookie_0", e->cookie, 0);
    expect("ev1_len_16", e->len, 16);
    check("ev1_name", str_eq(buf + 16, "abc"), 0);
    check("ev1_nul_padded", buf[19] == 0 && buf[31] == 0, buf[31]);

    /* 16-char name: len 32 (the NUL needs another 16). */
    expect("read_47_einval", a_read(fd, buf, 47), -EINVAL);
    memset(buf, 0x55, sizeof buf);
    expect("read_whole_fit_only", a_read(fd, buf, 48 + 31), 48);
    expect("ev2_len_32", e->len, 32);
    check("ev2_name", str_eq(buf + 16, "0123456789abcdef"), 0);
    expect("ev2_terminator", buf[32], 0);

    expect("fionread_32", queued(fd), 32);
    expect("read_rest", a_read(fd, buf, sizeof buf), 32);
    expect("ev3_mask_delete", e->mask, IN_DELETE);
    expect("drained_eagain", a_read(fd, buf, sizeof buf), -EAGAIN);
    expect("drained_poll_0", poll_in(fd), 0);

    a_unlink(DIR "/0123456789abcdef");
    a_rmdir(DIR);
    return finish();
}
