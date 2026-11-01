/*
 * wl-proto: the protocols a toolkit needs, against Poly's compositor on
 * KMS (ADR 0116). Like wl-client.c it speaks the wire protocol itself,
 * written from the protocol XML (wayland.xml, xdg-shell.xml,
 * presentation-time.xml, linux-dmabuf-v1.xml) and the DRM uapi headers:
 * no libwayland, no libdrm. Linked statically with the host's C library.
 *
 * Run by `poly-compositor --kms --exec /bin/wl-proto`, one scenario after
 * the other, each in a window of its own:
 *
 *   vsync      60 frames, each with a frame callback and presentation
 *              feedback: every frame presented with the vsync, hardware
 *              clock and completion flags, at vblanks (times a whole
 *              number of refresh periods apart, sequences counting them),
 *              after its commit and before it was heard of.
 *   subsurface a synchronized subsurface shown with its parent's commit,
 *              above it, at its position.
 *   popup      a popup whose positioner would put it past the right edge
 *              of the screen, flipped back by the constraint adjustment.
 *   dmabuf     a dumb buffer of /dev/dri/card0 shared as a PRIME dma-buf
 *              through zwp_linux_dmabuf_v1, composed like shared memory.
 *   clipboard  a second client (forked) is offered this one's selection
 *              when it gets the keyboard focus, and pastes it: the text
 *              goes through a pipe from one client to the other.
 *
 * After each scenario's frame is on the screen it asks the compositor
 * for a picture (SIGUSR1; $POLY_COMPOSITOR_DUMP) and counts its colours,
 * then prints "wl-proto: show <name>" and waits for a line on standard
 * input: the boot test takes a screenshot through QMP and answers.
 *
 * Output: "ok <check>" or "not ok <check> (...)" per check, "summary ...",
 * and "wl-proto: ok" if all passed (exit status 0).
 */
#define _GNU_SOURCE /* pipe2 */
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
#include <drm/drm.h>
#include <drm/drm_mode.h>

#define FORMAT_XRGB8888 1
#define FOURCC(a, b, c, d) ((uint32_t)(a) | (uint32_t)(b) << 8 | (uint32_t)(c) << 16 | (uint32_t)(d) << 24)
#define DRM_XRGB8888 FOURCC('X', 'R', '2', '4')

/* presentation-time.xml: wp_presentation_feedback.kind. */
#define KIND_VSYNC 0x1
#define KIND_HW_CLOCK 0x2
#define KIND_HW_COMPLETION 0x4

/* The scenarios' colours. */
#define VSYNC_BG 0x3a5f8au
#define SUB_PARENT 0x2f7d6du
#define SUB_CHILD 0xd04a4au
#define POPUP_PARENT 0x6d5a9eu
#define POPUP_COLOUR 0xe0c040u
#define DMABUF_COLOUR 0x40b0e0u
#define CLIP_TEXT "Poly clipboard: \xc3\xa4\xc3\xb6\xc3\xbc 0123456789"

static int passed, failed;

static void check(int ok, const char *what, const char *fmt, ...) __attribute__((format(printf, 3, 4)));

static void check(int ok, const char *what, const char *fmt, ...) {
    if (ok) {
        passed++;
        printf("ok %s\n", what);
    } else {
        char got[256];
        va_list ap;
        va_start(ap, fmt);
        vsnprintf(got, sizeof got, fmt, ap);
        va_end(ap);
        failed++;
        printf("not ok %s (%s)\n", what, got);
    }
    fflush(stdout);
}

static void die(const char *what) {
    printf("not ok %s (%s)\n", what, strerror(errno));
    printf("summary %d passed, %d failed\n", passed, failed + 1);
    exit(1);
}

static uint64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000u + (uint64_t)ts.tv_nsec;
}

/* ---- The connection ---------------------------------------------------- */

struct conn {
    int fd;
    uint8_t out[16384];
    size_t out_len;
    uint8_t in[65536];
    size_t in_len;
    uint32_t next_id;
};

static struct conn conn;

static void connect_display(void) {
    const char *dir = getenv("XDG_RUNTIME_DIR");
    const char *name = getenv("WAYLAND_DISPLAY");
    struct sockaddr_un addr = {.sun_family = AF_UNIX};
    if (!dir) {
        errno = ENOENT;
        die("XDG_RUNTIME_DIR");
    }
    snprintf(addr.sun_path, sizeof addr.sun_path, "%s/%s", dir, name ? name : "wayland-0");
    conn.fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (conn.fd < 0 || connect(conn.fd, (struct sockaddr *)&addr, sizeof addr) < 0)
        die("connect to the compositor");
    conn.out_len = conn.in_len = 0;
    conn.next_id = 2;
}

static uint32_t new_id(void) { return conn.next_id++; }

static uint32_t body[1024];
static size_t body_len;

static void begin(void) { body_len = 0; }
static void put(uint32_t v) { body[body_len++] = v; }

static void put_string(const char *s) {
    size_t n = strlen(s) + 1;
    put((uint32_t)n);
    memset(&body[body_len], 0, (n + 3) & ~(size_t)3);
    memcpy(&body[body_len], s, n);
    body_len += (n + 3) / 4;
}

static void send_bytes(int fd_to_pass);

static void end(uint32_t object, uint16_t opcode) {
    uint32_t size = 8 + 4 * (uint32_t)body_len;
    uint32_t header[2] = {object, size << 16 | opcode};
    if (conn.out_len + size > sizeof conn.out)
        send_bytes(-1);
    memcpy(conn.out + conn.out_len, header, 8);
    memcpy(conn.out + conn.out_len + 8, body, 4 * body_len);
    conn.out_len += size;
}

/* Send what is queued, with `fd_to_pass` (or none: -1). */
static void send_bytes(int fd_to_pass) {
    struct iovec iov = {.iov_base = conn.out, .iov_len = conn.out_len};
    union {
        struct cmsghdr align;
        char buf[CMSG_SPACE(sizeof(int))];
    } control;
    struct msghdr msg = {.msg_iov = &iov, .msg_iovlen = 1};
    if (fd_to_pass >= 0) {
        msg.msg_control = control.buf;
        msg.msg_controllen = sizeof control.buf;
        struct cmsghdr *cmsg = CMSG_FIRSTHDR(&msg);
        cmsg->cmsg_level = SOL_SOCKET;
        cmsg->cmsg_type = SCM_RIGHTS;
        cmsg->cmsg_len = CMSG_LEN(sizeof(int));
        memcpy(CMSG_DATA(cmsg), &fd_to_pass, sizeof(int));
    }
    while (iov.iov_len > 0) {
        ssize_t n = sendmsg(conn.fd, &msg, MSG_NOSIGNAL);
        if (n < 0 && errno == EINTR)
            continue;
        if (n < 0)
            die("sendmsg");
        iov.iov_base = (char *)iov.iov_base + n;
        iov.iov_len -= (size_t)n;
        msg.msg_control = NULL;
        msg.msg_controllen = 0;
    }
    conn.out_len = 0;
}

static void flush(void) {
    if (conn.out_len)
        send_bytes(-1);
}

/* A request carrying a file descriptor: sent at once with it. */
static void end_fd(uint32_t object, uint16_t opcode, int fd) {
    flush();
    end(object, opcode);
    send_bytes(fd);
}

struct event {
    uint32_t object;
    uint16_t opcode;
    const uint32_t *args;
    uint32_t len;
    int fd; /* a descriptor that came with the message, or -1 */
};

static uint32_t event_copy[65536 / 4];
static int fd_queue[16];
static int fd_count;

/* The next event, waiting up to `ms` for it; 0 on timeout or EOF. */
static int next_event(struct event *e, int ms) {
    for (;;) {
        if (conn.in_len >= 8) {
            uint32_t header[2];
            memcpy(header, conn.in, 8);
            uint32_t size = header[1] >> 16;
            if (size >= 8 && conn.in_len >= size) {
                memcpy(event_copy, conn.in, size);
                memmove(conn.in, conn.in + size, conn.in_len - size);
                conn.in_len -= size;
                e->object = header[0];
                e->opcode = (uint16_t)header[1];
                e->args = event_copy + 2;
                e->len = size - 8;
                e->fd = -1;
                return 1;
            }
        }
        struct pollfd p = {.fd = conn.fd, .events = POLLIN};
        int r = poll(&p, 1, ms);
        if (r < 0 && errno == EINTR)
            continue;
        if (r <= 0)
            return 0;
        struct iovec iov = {.iov_base = conn.in + conn.in_len, .iov_len = sizeof conn.in - conn.in_len};
        union {
            struct cmsghdr align;
            char buf[CMSG_SPACE(sizeof(int) * 8)];
        } control;
        struct msghdr msg = {.msg_iov = &iov, .msg_iovlen = 1, .msg_control = control.buf,
                             .msg_controllen = sizeof control.buf};
        ssize_t n = recvmsg(conn.fd, &msg, MSG_CMSG_CLOEXEC);
        if (n < 0 && errno == EINTR)
            continue;
        if (n <= 0)
            return 0;
        for (struct cmsghdr *c = CMSG_FIRSTHDR(&msg); c; c = CMSG_NXTHDR(&msg, c)) {
            if (c->cmsg_level == SOL_SOCKET && c->cmsg_type == SCM_RIGHTS) {
                int count = (int)((c->cmsg_len - CMSG_LEN(0)) / sizeof(int));
                for (int i = 0; i < count && fd_count < 16; i++)
                    memcpy(&fd_queue[fd_count++], CMSG_DATA(c) + i * sizeof(int), sizeof(int));
            }
        }
        conn.in_len += (size_t)n;
    }
}

/* The oldest descriptor received and not taken yet. */
static int take_fd(void) {
    if (fd_count == 0)
        return -1;
    int fd = fd_queue[0];
    memmove(fd_queue, fd_queue + 1, sizeof(int) * (size_t)--fd_count);
    return fd;
}

static const char *arg_string(const struct event *e, uint32_t at) {
    return e->args[at] ? (const char *)&e->args[at + 1] : "";
}

/* ---- Globals and state ---------------------------------------------------- */

static uint32_t registry;
static struct {
    uint32_t compositor, shm, wm_base, subcompositor, presentation, seat, data_manager, dmabuf, output;
} gname;
static uint32_t compositor, shm, wm_base, subcompositor, presentation, seat, data_manager, dmabuf;
static int protocol_error;
static uint32_t sync_cb;
static int sync_done;
static int dmabuf_xrgb, dmabuf_linear;

/* What a scenario waits for. */
static uint32_t watch_xdg, watch_frame, watch_feedback, watch_popup, watch_params, watch_device, watch_source;
static uint32_t configure_serial;
static int configured, frame_done;
static int32_t popup_x, popup_y, popup_w, popup_h;
static int popup_configured;
/* Presentation feedback of the vsync scenario. */
static int fb_state; /* 0 waiting, 1 presented, 2 discarded */
static uint64_t fb_time;
static uint32_t fb_refresh, fb_flags;
static uint64_t fb_seq;
/* The clipboard. */
static uint32_t current_offer, selection_offer;
static int offer_has_text, selection_seen, source_cancelled, sends;

static void handle(const struct event *e);

static int wait_for(int *flag, int ms) {
    struct event e;
    flush();
    uint64_t end = now_ns() + (uint64_t)ms * 1000000u;
    while (!*flag && !protocol_error) {
        int64_t left = (int64_t)(end - now_ns()) / 1000000;
        if (left <= 0 || !next_event(&e, (int)left))
            return 0;
        handle(&e);
    }
    return *flag;
}

static void roundtrip(void) {
    sync_cb = new_id();
    sync_done = 0;
    begin();
    put(sync_cb);
    end(1, 0);
    if (!wait_for(&sync_done, 20000))
        die("roundtrip");
}

static uint32_t bind_global(uint32_t name, const char *iface, uint32_t version) {
    uint32_t id = new_id();
    begin();
    put(name);
    put_string(iface);
    put(version);
    put(id);
    end(registry, 0);
    return id;
}

static void handle(const struct event *e) {
    if (e->object == 1 && e->opcode == 0) {
        protocol_error = 1;
        printf("wl-proto: protocol error on object %u, code %u: %s\n", e->args[0], e->args[1], arg_string(e, 2));
    } else if (e->object == registry && e->opcode == 0) {
        const char *iface = arg_string(e, 1);
        uint32_t name = e->args[0];
        if (!strcmp(iface, "wl_compositor"))
            gname.compositor = name;
        else if (!strcmp(iface, "wl_shm"))
            gname.shm = name;
        else if (!strcmp(iface, "xdg_wm_base"))
            gname.wm_base = name;
        else if (!strcmp(iface, "wl_subcompositor"))
            gname.subcompositor = name;
        else if (!strcmp(iface, "wp_presentation"))
            gname.presentation = name;
        else if (!strcmp(iface, "wl_seat"))
            gname.seat = name;
        else if (!strcmp(iface, "wl_data_device_manager"))
            gname.data_manager = name;
        else if (!strcmp(iface, "zwp_linux_dmabuf_v1"))
            gname.dmabuf = name;
        else if (!strcmp(iface, "wl_output"))
            gname.output = name;
    } else if (e->object == sync_cb && e->opcode == 0) {
        sync_done = 1;
    } else if (e->object == wm_base && e->opcode == 0) {
        begin();
        put(e->args[0]);
        end(wm_base, 3); /* pong */
    } else if (e->object == watch_xdg && e->opcode == 0) {
        configure_serial = e->args[0];
        configured = 1;
    } else if (e->object == watch_frame && watch_frame && e->opcode == 0) {
        frame_done = 1;
    } else if (e->object == watch_feedback && watch_feedback && e->opcode == 1) {
        fb_time = ((uint64_t)e->args[0] << 32 | e->args[1]) * 1000000000u + e->args[2];
        fb_refresh = e->args[3];
        fb_seq = (uint64_t)e->args[4] << 32 | e->args[5];
        fb_flags = e->args[6];
        fb_state = 1;
    } else if (e->object == watch_feedback && watch_feedback && e->opcode == 2) {
        fb_state = 2;
    } else if (e->object == watch_popup && watch_popup && e->opcode == 0) {
        popup_x = (int32_t)e->args[0];
        popup_y = (int32_t)e->args[1];
        popup_w = (int32_t)e->args[2];
        popup_h = (int32_t)e->args[3];
        popup_configured = 1;
    } else if (e->object == dmabuf && dmabuf && e->opcode == 1) {
        if (e->args[0] == DRM_XRGB8888) {
            dmabuf_xrgb = 1;
            if (e->args[1] == 0 && e->args[2] == 0)
                dmabuf_linear = 1;
        }
    } else if (e->object == watch_device && watch_device && e->opcode == 0) {
        current_offer = e->args[0]; /* data_offer: a new wl_data_offer */
        offer_has_text = 0;
    } else if (e->object == current_offer && current_offer && e->opcode == 0) {
        if (!strcmp(arg_string(e, 0), "text/plain;charset=utf-8"))
            offer_has_text = 1;
    } else if (e->object == watch_device && watch_device && e->opcode == 5) {
        selection_offer = e->args[0];
        selection_seen = 1;
    } else if (e->object == watch_source && watch_source && e->opcode == 1) {
        /* send: write the text into the pipe and close it. */
        int fd = take_fd();
        if (fd >= 0) {
            const char *text = CLIP_TEXT;
            ssize_t w = write(fd, text, strlen(text));
            (void)w;
            close(fd);
            sends++;
        }
    } else if (e->object == watch_source && watch_source && e->opcode == 2) {
        source_cancelled = 1;
    }
}

static void setup(void) {
    connect_display();
    registry = new_id();
    begin();
    put(registry);
    end(1, 1); /* get_registry */
    roundtrip();
    if (!gname.compositor || !gname.shm || !gname.wm_base)
        die("the core globals");
    compositor = bind_global(gname.compositor, "wl_compositor", 6);
    shm = bind_global(gname.shm, "wl_shm", 1);
    wm_base = bind_global(gname.wm_base, "xdg_wm_base", 6);
    if (gname.subcompositor)
        subcompositor = bind_global(gname.subcompositor, "wl_subcompositor", 1);
    if (gname.presentation)
        presentation = bind_global(gname.presentation, "wp_presentation", 1);
    if (gname.seat)
        seat = bind_global(gname.seat, "wl_seat", 7);
    if (gname.data_manager)
        data_manager = bind_global(gname.data_manager, "wl_data_device_manager", 3);
    if (gname.dmabuf)
        dmabuf = bind_global(gname.dmabuf, "zwp_linux_dmabuf_v1", 3);
    roundtrip();
}

/* ---- Buffers and windows ------------------------------------------------------- */

struct shm_buffer {
    uint32_t id;
    uint32_t *pixels;
    int w, h;
};

/* A w x h xrgb8888 buffer filled with `colour`, from a file in /dev/shm. */
static struct shm_buffer make_buffer(int w, int h, uint32_t colour) {
    struct shm_buffer b = {.w = w, .h = h};
    char path[64];
    snprintf(path, sizeof path, "/dev/shm/wl-proto-%d-%u", getpid(), conn.next_id);
    int fd = open(path, O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
    if (fd < 0)
        die("a shared memory file");
    unlink(path);
    size_t size = (size_t)w * (size_t)h * 4;
    if (ftruncate(fd, (off_t)size) < 0)
        die("ftruncate");
    b.pixels = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (b.pixels == MAP_FAILED)
        die("mmap");
    for (size_t i = 0; i < (size_t)w * (size_t)h; i++)
        b.pixels[i] = colour;
    uint32_t pool = new_id();
    begin();
    put(pool);
    put((uint32_t)size);
    end_fd(shm, 0, fd); /* create_pool(id, fd, size) */
    close(fd);
    b.id = new_id();
    begin();
    put(b.id);
    put(0);
    put((uint32_t)w);
    put((uint32_t)h);
    put((uint32_t)w * 4);
    put(FORMAT_XRGB8888);
    end(pool, 0);
    begin();
    end(pool, 1); /* destroy the pool: the buffer keeps it */
    return b;
}

static uint32_t make_surface(void) {
    uint32_t s = new_id();
    begin();
    put(s);
    end(compositor, 0);
    return s;
}

static void attach(uint32_t surface, uint32_t buffer) {
    begin();
    put(buffer);
    put(0);
    put(0);
    end(surface, 1);
    begin();
    put(0);
    put(0);
    put(0x7fffffff);
    put(0x7fffffff);
    end(surface, 9); /* damage_buffer */
}

static void commit(uint32_t surface) {
    begin();
    end(surface, 6);
}

/* A frame callback on `surface` (watched). */
static void request_frame(uint32_t surface) {
    watch_frame = new_id();
    frame_done = 0;
    begin();
    put(watch_frame);
    end(surface, 3);
}

struct window {
    uint32_t surface, xdg, toplevel;
};

/* A toplevel up to its first acked configure (no buffer yet). */
static struct window make_toplevel(const char *title) {
    struct window w;
    w.surface = make_surface();
    w.xdg = new_id();
    begin();
    put(w.xdg);
    put(w.surface);
    end(wm_base, 2);
    w.toplevel = new_id();
    begin();
    put(w.toplevel);
    end(w.xdg, 1);
    begin();
    put_string(title);
    end(w.toplevel, 2);
    watch_xdg = w.xdg;
    configured = 0;
    commit(w.surface);
    if (!wait_for(&configured, 20000))
        die("the first configure");
    begin();
    put(configure_serial);
    end(w.xdg, 4);
    return w;
}

/* Show `buffer` in the window and wait until that frame is on the screen. */
static int show(uint32_t surface, uint32_t buffer) {
    attach(surface, buffer);
    request_frame(surface);
    commit(surface);
    return wait_for(&frame_done, 20000);
}

static void destroy_window(struct window *w) {
    begin();
    end(w->toplevel, 0);
    begin();
    end(w->xdg, 0);
    begin();
    end(w->surface, 0);
    roundtrip();
}

/* ---- Pictures --------------------------------------------------------------------- */

/* Ask the compositor for a picture (SIGUSR1) and count the pixels of each
 * of `colours`. Returns 0 if there is no picture. */
static int count_colours(const uint32_t *colours, long *counts, int n) {
    const char *path = getenv("POLY_COMPOSITOR_DUMP");
    struct stat st;
    for (int i = 0; i < n; i++)
        counts[i] = 0;
    if (!path)
        return 0;
    unlink(path);
    kill(getppid(), SIGUSR1);
    for (int waited = 0; stat(path, &st) < 0 && waited < 20000; waited += 20)
        usleep(20000);
    FILE *f = fopen(path, "rb");
    int w = 0, h = 0, max = 0;
    if (!f || fscanf(f, "P6 %d %d %d", &w, &h, &max) != 3 || fgetc(f) != '\n') {
        if (f)
            fclose(f);
        return 0;
    }
    unsigned char px[3];
    for (long i = 0; i < (long)w * h && fread(px, 1, 3, f) == 3; i++) {
        uint32_t c = (uint32_t)px[0] << 16 | (uint32_t)px[1] << 8 | px[2];
        for (int k = 0; k < n; k++)
            if (c == colours[k])
                counts[k]++;
    }
    fclose(f);
    return 1;
}

/* Tell the boot test what is on the screen and wait for its answer. */
static void shown(const char *name) {
    printf("wl-proto: show %s\n", name);
    fflush(stdout);
    char line[64];
    struct pollfd p = {.fd = 0, .events = POLLIN};
    /* Without anyone to answer (run by hand), go on after a while. */
    if (poll(&p, 1, 30000) > 0 && fgets(line, sizeof line, stdin)) {
    }
}

/* ---- The scenarios ------------------------------------------------------------- */

#define VSYNC_FRAMES 60

static void scenario_vsync(void) {
    if (!presentation) {
        check(0, "vsync: wp_presentation is offered", "no global");
        return;
    }
    struct window w = make_toplevel("wl-proto vsync");
    struct shm_buffer b = make_buffer(160, 100, VSYNC_BG);
    show(w.surface, b.id);
    uint64_t times[VSYNC_FRAMES], seqs[VSYNC_FRAMES];
    uint32_t refresh = 0;
    int presented = 0, flags_ok = 1, order_ok = 1;
    for (int i = 0; i < VSYNC_FRAMES; i++) {
        /* A small bar moves along the window: a new frame every time. */
        for (int y = 40; y < 60; y++)
            for (int x = 0; x < 160; x++)
                b.pixels[y * 160 + x] = (x / 8 == i % 20) ? 0xffffffu : VSYNC_BG;
        watch_feedback = new_id();
        fb_state = 0;
        begin();
        put(w.surface);
        put(watch_feedback);
        end(presentation, 1);
        uint64_t committed = now_ns();
        if (!show(w.surface, b.id) || !wait_for(&fb_state, 20000))
            break;
        uint64_t heard = now_ns();
        if (fb_state != 1)
            continue;
        if ((fb_flags & (KIND_VSYNC | KIND_HW_CLOCK | KIND_HW_COMPLETION)) !=
            (KIND_VSYNC | KIND_HW_CLOCK | KIND_HW_COMPLETION))
            flags_ok = 0;
        if (fb_time < committed || fb_time > heard)
            order_ok = 0;
        times[presented] = fb_time;
        seqs[presented] = fb_seq;
        refresh = fb_refresh;
        presented++;
    }
    watch_feedback = 0;
    check(presented == VSYNC_FRAMES, "vsync: every frame was presented", "%d of %d", presented, VSYNC_FRAMES);
    check(flags_ok, "vsync: presented at vblank, by the display's clock and completion", "flags %#x", fb_flags);
    check(order_ok, "vsync: each frame shown after its commit and before it was heard of", "out of order");
    check(refresh > 1000000 && refresh < 100000000, "vsync: the refresh period is known", "%u ns", refresh);
    int whole = 1, seq_ok = 1, at_next = 0;
    uint64_t span = presented > 1 ? times[presented - 1] - times[0] : 0;
    for (int i = 1; i < presented && refresh; i++) {
        uint64_t dt = times[i] - times[i - 1];
        uint64_t k = (dt + refresh / 2) / refresh;
        int64_t off = (int64_t)dt - (int64_t)(k * refresh);
        if (k == 0 || off > 1000000 || off < -1000000)
            whole = 0;
        if (seqs[i] - seqs[i - 1] != k)
            seq_ok = 0;
        if (k == 1)
            at_next++;
    }
    check(whole, "vsync: frames are whole refresh periods apart", "not on vblanks");
    check(seq_ok, "vsync: vblank sequences count the periods between frames", "sequences disagree");
    uint64_t mean = presented > 1 ? span / (uint64_t)(presented - 1) : 0;
    check(presented > 1 && mean + 100000 >= refresh, "vsync: no faster than the display", "mean %llu ns",
          (unsigned long long)mean);
    printf("wl-proto: vsync: %d frames in %llu ms, refresh %u ns (%u.%03u Hz), mean interval %llu ns, %d at the very "
           "next vblank\n",
           presented, (unsigned long long)(span / 1000000), refresh, refresh ? 1000000000u / refresh : 0,
           refresh ? (uint32_t)(1000000000000ull / refresh % 1000) : 0, (unsigned long long)mean, at_next);
    uint32_t colours[1] = {VSYNC_BG};
    long counts[1];
    if (count_colours(colours, counts, 1))
        check(counts[0] > 160 * 80, "vsync: the window is on the screen", "%ld pixels", counts[0]);
    shown("vsync");
    destroy_window(&w);
}

static void scenario_subsurface(void) {
    if (!subcompositor) {
        check(0, "subsurface: wl_subcompositor is offered", "no global");
        return;
    }
    struct window w = make_toplevel("wl-proto subsurface");
    struct shm_buffer parent = make_buffer(200, 150, SUB_PARENT);
    struct shm_buffer child_buf = make_buffer(60, 40, SUB_CHILD);
    uint32_t child = make_surface();
    uint32_t sub = new_id();
    begin();
    put(sub);
    put(child);
    put(w.surface);
    end(subcompositor, 1);
    begin();
    put(20);
    put(30);
    end(sub, 1); /* set_position(20, 30) */
    /* Synchronized: the child's commit waits for the parent's. */
    attach(child, child_buf.id);
    commit(child);
    if (!show(w.surface, parent.id)) {
        check(0, "subsurface: the frame was shown", "no frame callback");
        return;
    }
    uint32_t colours[2] = {SUB_PARENT, SUB_CHILD};
    long counts[2];
    if (count_colours(colours, counts, 2)) {
        check(counts[1] == 60 * 40, "subsurface: shown with its parent's commit, whole and above it", "%ld pixels",
              counts[1]);
        check(counts[0] == 200 * 150 - 60 * 40, "subsurface: the parent around it", "%ld pixels", counts[0]);
    }
    shown("subsurface");
    begin();
    end(sub, 0);
    begin();
    end(child, 0);
    destroy_window(&w);
}

static void scenario_popup(void) {
    struct window w = make_toplevel("wl-proto popup");
    struct shm_buffer parent = make_buffer(240, 160, POPUP_PARENT);
    show(w.surface, parent.id);
    uint32_t positioner = new_id();
    begin();
    put(positioner);
    end(wm_base, 1);
    begin();
    put(120);
    put(80);
    end(positioner, 1); /* set_size */
    /* Anchored far right of the window: unflipped it would leave the
     * screen, so flip_x puts it to the anchor's left instead. */
    begin();
    put(2000);
    put(20);
    put(1);
    put(1);
    end(positioner, 2); /* set_anchor_rect */
    begin();
    put(8);
    end(positioner, 3); /* anchor bottom_right */
    begin();
    put(8);
    end(positioner, 4); /* gravity bottom_right */
    begin();
    put(4 | 1);
    end(positioner, 5); /* flip_x, slide_x */
    uint32_t ps = make_surface();
    uint32_t px = new_id();
    begin();
    put(px);
    put(ps);
    end(wm_base, 2);
    watch_popup = new_id();
    popup_configured = 0;
    begin();
    put(watch_popup);
    put(w.xdg);
    put(positioner);
    end(px, 2); /* get_popup */
    watch_xdg = px;
    configured = 0;
    commit(ps);
    wait_for(&configured, 20000);
    check(popup_configured, "popup: configured", "no xdg_popup.configure");
    printf("wl-proto: popup configured at %d,%d size %dx%d\n", popup_x, popup_y, popup_w, popup_h);
    /* Unadjusted it would start at the anchor rectangle's right, 2001. */
    check(popup_configured && popup_x != 2001 && popup_x + popup_w <= 1280,
          "popup: kept on the screen by the constraint adjustment", "x %d", popup_x);
    begin();
    put(configure_serial);
    end(px, 4);
    struct shm_buffer pb = make_buffer(popup_w > 0 ? popup_w : 120, popup_h > 0 ? popup_h : 80, POPUP_COLOUR);
    if (!show(ps, pb.id)) {
        check(0, "popup: the frame was shown", "no frame callback");
        return;
    }
    uint32_t colours[1] = {POPUP_COLOUR};
    long counts[1];
    if (count_colours(colours, counts, 1))
        check(counts[0] == (long)pb.w * pb.h, "popup: shown whole", "%ld pixels of %d", counts[0], pb.w * pb.h);
    shown("popup");
    begin();
    end(watch_popup, 0);
    watch_popup = 0;
    begin();
    end(px, 0);
    begin();
    end(ps, 0);
    destroy_window(&w);
}

static int xioctl(int fd, unsigned long req, void *arg) {
    int r;
    do
        r = ioctl(fd, req, arg);
    while (r < 0 && errno == EINTR);
    return r;
}

static void scenario_dmabuf(void) {
    if (!dmabuf) {
        check(0, "dmabuf: zwp_linux_dmabuf_v1 is offered", "no global (not on KMS?)");
        return;
    }
    check(dmabuf_xrgb && dmabuf_linear, "dmabuf: XRGB8888 with the linear modifier is offered", "not listed");
    int card = open("/dev/dri/card0", O_RDWR | O_CLOEXEC);
    if (card < 0) {
        check(0, "dmabuf: /dev/dri/card0 opens", "%s", strerror(errno));
        return;
    }
    const int w = 128, h = 96;
    struct drm_mode_create_dumb c = {.width = w, .height = h, .bpp = 32};
    if (xioctl(card, DRM_IOCTL_MODE_CREATE_DUMB, &c) < 0) {
        check(0, "dmabuf: a dumb buffer (not the DRM master)", "%s", strerror(errno));
        return;
    }
    struct drm_mode_map_dumb m = {.handle = c.handle};
    uint32_t *pixels = MAP_FAILED;
    if (xioctl(card, DRM_IOCTL_MODE_MAP_DUMB, &m) == 0)
        pixels = mmap(NULL, c.size, PROT_READ | PROT_WRITE, MAP_SHARED, card, (off_t)m.offset);
    if (pixels == MAP_FAILED) {
        check(0, "dmabuf: the dumb buffer maps", "%s", strerror(errno));
        return;
    }
    for (int y = 0; y < h; y++)
        for (int x = 0; x < w; x++)
            pixels[y * (c.pitch / 4) + x] = DMABUF_COLOUR;
    struct drm_prime_handle ph = {.handle = c.handle, .flags = DRM_CLOEXEC | DRM_RDWR};
    if (xioctl(card, DRM_IOCTL_PRIME_HANDLE_TO_FD, &ph) < 0) {
        check(0, "dmabuf: PRIME export", "%s", strerror(errno));
        return;
    }
    check(1, "dmabuf: a dumb buffer exported as a dma-buf", "%s", "");
    uint32_t params = new_id();
    begin();
    put(params);
    end(dmabuf, 1); /* create_params */
    begin();
    put(0);       /* plane_idx */
    put(0);       /* offset */
    put(c.pitch); /* stride */
    put(0);       /* modifier hi: DRM_FORMAT_MOD_LINEAR */
    put(0);
    end_fd(params, 1, ph.fd); /* add(fd, ...) */
    close(ph.fd);
    uint32_t buffer = new_id();
    begin();
    put(buffer);
    put((uint32_t)w);
    put((uint32_t)h);
    put(DRM_XRGB8888);
    put(0);
    end(params, 3); /* create_immed */
    struct window win = make_toplevel("wl-proto dmabuf");
    if (!show(win.surface, buffer) || protocol_error) {
        check(0, "dmabuf: the frame was shown", "no frame callback");
        return;
    }
    uint32_t colours[1] = {DMABUF_COLOUR};
    long counts[1];
    if (count_colours(colours, counts, 1))
        check(counts[0] == (long)w * h, "dmabuf: the client's dma-buf is composed", "%ld pixels of %d", counts[0],
              w * h);
    shown("dmabuf");
    destroy_window(&win);
    munmap(pixels, c.size);
    close(card);
}

/* The forked second client: maps a window (getting the keyboard focus and
 * with it the selection) and pastes. Exit status 0 if it got the text. */
static int paste_client(void) {
    passed = failed = 0;
    /* The copier's object IDs mean nothing on this connection. */
    watch_xdg = watch_frame = watch_feedback = watch_popup = watch_params = watch_device = watch_source = 0;
    current_offer = selection_offer = 0;
    memset(&gname, 0, sizeof gname);
    fd_count = 0;
    setup();
    if (!data_manager || !seat)
        return 2;
    watch_device = new_id();
    begin();
    put(watch_device);
    put(seat);
    end(data_manager, 1);
    struct window w = make_toplevel("wl-proto paste");
    struct shm_buffer b = make_buffer(80, 60, 0x808080u);
    show(w.surface, b.id);
    if (!wait_for(&selection_seen, 20000) || !selection_offer || selection_offer != current_offer || !offer_has_text) {
        printf("wl-proto: paste: no selection offer with text (offer %u, text %d)\n", selection_offer, offer_has_text);
        return 3;
    }
    int p[2];
    if (pipe2(p, O_CLOEXEC) < 0)
        return 4;
    begin();
    put_string("text/plain;charset=utf-8");
    end_fd(selection_offer, 1, p[1]); /* receive(mime, fd) */
    close(p[1]);
    char got[256];
    size_t n = 0;
    for (;;) {
        struct pollfd pf = {.fd = p[0], .events = POLLIN};
        if (poll(&pf, 1, 20000) <= 0)
            break;
        ssize_t r = read(p[0], got + n, sizeof got - 1 - n);
        if (r <= 0)
            break;
        n += (size_t)r;
    }
    got[n] = 0;
    printf("wl-proto: pasted \"%s\"\n", got);
    fflush(stdout);
    destroy_window(&w);
    return strcmp(got, CLIP_TEXT) == 0 ? 0 : 5;
}

static void scenario_clipboard(void) {
    if (!data_manager || !seat) {
        check(0, "clipboard: wl_data_device_manager and wl_seat are offered", "no globals");
        return;
    }
    uint32_t device = new_id();
    begin();
    put(device);
    put(seat);
    end(data_manager, 1);
    struct window w = make_toplevel("wl-proto copy");
    struct shm_buffer b = make_buffer(80, 60, 0x404040u);
    show(w.surface, b.id);
    watch_source = new_id();
    begin();
    put(watch_source);
    end(data_manager, 0);
    begin();
    put_string("text/plain;charset=utf-8");
    end(watch_source, 0);
    begin();
    put(watch_source);
    put(0);
    end(device, 1); /* set_selection */
    roundtrip();
    fflush(stdout);
    pid_t pid = fork();
    if (pid == 0) {
        close(conn.fd);
        exit(paste_client());
    }
    if (pid < 0) {
        check(0, "clipboard: a second client", "%s", strerror(errno));
        return;
    }
    /* Serve `send` until the paster is done. */
    int status = -1;
    for (int i = 0; i < 3000; i++) {
        struct event e;
        flush();
        while (next_event(&e, 20))
            handle(&e);
        if (waitpid(pid, &status, WNOHANG) == pid)
            break;
    }
    check(sends == 1, "clipboard: the source was asked to send its text once", "%d sends", sends);
    check(WIFEXITED(status) && WEXITSTATUS(status) == 0,
          "clipboard: the second client was offered the selection and pasted it", "status %d",
          WIFEXITED(status) ? WEXITSTATUS(status) : -1);
    shown("clipboard");
    destroy_window(&w);
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    setup();
    const char *only = argc > 1 ? argv[1] : "all";
    int all = !strcmp(only, "all");
    if (all || !strcmp(only, "vsync"))
        scenario_vsync();
    if (all || !strcmp(only, "subsurface"))
        scenario_subsurface();
    if (all || !strcmp(only, "popup"))
        scenario_popup();
    if (all || !strcmp(only, "dmabuf"))
        scenario_dmabuf();
    if (all || !strcmp(only, "clipboard"))
        scenario_clipboard();
    roundtrip();
    check(!protocol_error, "no protocol error", "the compositor cut us off");
    printf("summary %d passed, %d failed\n", passed, failed);
    if (failed == 0)
        printf("wl-proto: ok\n");
    return failed ? 1 : 0;
}
