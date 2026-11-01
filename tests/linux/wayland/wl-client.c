/*
 * wl-client: a minimal Wayland client for Poly's compositor boot test
 * (ADR 0106). It speaks the wire protocol itself, written from the
 * protocol XML (wayland.xml, xdg-shell.xml, xdg-decoration-unstable-v1.xml)
 * and "The Wayland Protocol" book: no libwayland. Linked statically with
 * the host's C library.
 *
 * It connects to $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY, binds the globals,
 * makes an xdg_toplevel with a server-side decoration, acks its configure,
 * shares a 64x48 xrgb8888 buffer through a file in /dev/shm (the fd goes
 * over the socket with SCM_RIGHTS), commits it and waits for the frame
 * callback and the buffer's release. Run by poly-compositor --exec (which
 * sets POLY_COMPOSITOR_DUMP), it then asks the compositor for a picture of
 * the output (SIGUSR1) and finds its own pixels in it. A second connection
 * checks that a request to an unknown object is a protocol error.
 *
 * Output, one line per check: "ok <check>" or "not ok <check> (...)",
 * then "summary <passed> passed, <failed> failed" and, if all passed,
 * "wl-client: ok". Exit status 0 when every check passed.
 *
 * Not in tests/linux/ itself: the diff runner compares those with real
 * Linux, which has no Poly compositor to talk to.
 */
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

/* The picture: left half one colour, right half another. */
#define WIDTH 64
#define HEIGHT 48
#define LEFT 0x2060c0u
#define RIGHT 0xe07020u

/* wl_shm.format, wl_display.error, zxdg_toplevel_decoration_v1.mode. */
#define FORMAT_ARGB8888 0
#define FORMAT_XRGB8888 1
#define ERROR_INVALID_OBJECT 0
#define MODE_SERVER_SIDE 2

static int passed, failed;

static void check(int ok, const char *what, const char *got) {
    if (ok) {
        passed++;
        printf("ok %s\n", what);
    } else {
        failed++;
        printf("not ok %s (%s)\n", what, got);
    }
}

static void die(const char *what) {
    printf("not ok %s (%s)\n", what, strerror(errno));
    printf("summary %d passed, %d failed\n", passed, failed + 1);
    exit(1);
}

/* ---- One connection ---------------------------------------------------- */

struct conn {
    int fd;
    uint8_t out[8192];
    size_t out_len;
    int out_fd; /* sent with the next flush, -1 for none */
    uint8_t in[65536];
    size_t in_len;
    uint32_t next_id;
};

static int connect_display(struct conn *c) {
    const char *dir = getenv("XDG_RUNTIME_DIR");
    const char *name = getenv("WAYLAND_DISPLAY");
    struct sockaddr_un addr = {.sun_family = AF_UNIX};
    if (!dir) {
        errno = ENOENT;
        return -1;
    }
    snprintf(addr.sun_path, sizeof addr.sun_path, "%s/%s", dir, name ? name : "wayland-0");
    c->fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (c->fd < 0)
        return -1;
    c->out_len = 0;
    c->in_len = 0;
    c->out_fd = -1;
    c->next_id = 2; /* 1 is wl_display */
    return connect(c->fd, (struct sockaddr *)&addr, sizeof addr);
}

static uint32_t new_id(struct conn *c) { return c->next_id++; }

/* A request being built. */
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

static void end(struct conn *c, uint32_t object, uint16_t opcode) {
    uint32_t size = 8 + 4 * (uint32_t)body_len;
    uint32_t header[2] = {object, size << 16 | opcode};
    memcpy(c->out + c->out_len, header, 8);
    memcpy(c->out + c->out_len + 8, body, 4 * body_len);
    c->out_len += size;
}

static void flush(struct conn *c) {
    struct iovec iov = {.iov_base = c->out, .iov_len = c->out_len};
    union {
        struct cmsghdr align;
        char buf[CMSG_SPACE(sizeof(int))];
    } control;
    struct msghdr msg = {.msg_iov = &iov, .msg_iovlen = 1};
    if (c->out_fd >= 0) {
        msg.msg_control = control.buf;
        msg.msg_controllen = sizeof control.buf;
        struct cmsghdr *cmsg = CMSG_FIRSTHDR(&msg);
        cmsg->cmsg_level = SOL_SOCKET;
        cmsg->cmsg_type = SCM_RIGHTS;
        cmsg->cmsg_len = CMSG_LEN(sizeof(int));
        memcpy(CMSG_DATA(cmsg), &c->out_fd, sizeof(int));
    }
    while (iov.iov_len > 0) {
        ssize_t n = sendmsg(c->fd, &msg, MSG_NOSIGNAL);
        if (n < 0 && errno == EINTR)
            continue;
        if (n < 0)
            die("sendmsg");
        iov.iov_base = (char *)iov.iov_base + n;
        iov.iov_len -= (size_t)n;
        msg.msg_control = NULL;
        msg.msg_controllen = 0;
    }
    c->out_len = 0;
    c->out_fd = -1;
}

/* An event: sender, opcode, arguments. */
struct event {
    uint32_t object;
    uint16_t opcode;
    const uint32_t *args;
    uint32_t len; /* bytes of arguments */
};

static uint32_t event_copy[65536 / 4];

/* The next event, waiting up to 10 seconds for it; 0 on timeout or EOF. */
static int next_event(struct conn *c, struct event *e) {
    for (;;) {
        if (c->in_len >= 8) {
            uint32_t header[2];
            memcpy(header, c->in, 8);
            uint32_t size = header[1] >> 16;
            if (size >= 8 && c->in_len >= size) {
                memcpy(event_copy, c->in, size);
                memmove(c->in, c->in + size, c->in_len - size);
                c->in_len -= size;
                e->object = header[0];
                e->opcode = (uint16_t)header[1];
                e->args = event_copy + 2;
                e->len = size - 8;
                return 1;
            }
        }
        struct pollfd p = {.fd = c->fd, .events = POLLIN};
        int r = poll(&p, 1, 10000);
        if (r < 0 && errno == EINTR)
            continue;
        if (r <= 0)
            return 0;
        ssize_t n = recv(c->fd, c->in + c->in_len, sizeof c->in - c->in_len, 0);
        if (n < 0 && errno == EINTR)
            continue;
        if (n <= 0)
            return 0;
        c->in_len += (size_t)n;
    }
}

/* The string argument at word `at` of an event. */
static const char *arg_string(const struct event *e, uint32_t at) {
    return e->args[at] ? (const char *)&e->args[at + 1] : "";
}

/* ---- The client's state -------------------------------------------------- */

static struct conn conn;

static struct {
    uint32_t compositor, shm, wm_base, decoration_manager, output;
} global_name;

static uint32_t registry, compositor, shm, wm_base, decoration_manager, output;
static uint32_t surface, xdg_surface, toplevel, decoration, pool, buffer, frame, sync_cb;
static int formats, sync_done, configured, released, frame_done, frame_deleted, entered, closed;
static uint32_t configure_serial, decoration_mode, preferred_scale = 1;
static int32_t mode_width, mode_height, output_scale = 1;
static int protocol_error;

static void handle(const struct event *e) {
    if (e->object == 1 && e->opcode == 0) {
        protocol_error = 1;
        printf("wl-client: protocol error on object %u, code %u: %s\n", e->args[0], e->args[1], arg_string(e, 2));
    } else if (e->object == 1 && e->opcode == 1) {
        if (e->args[0] == frame)
            frame_deleted = 1;
    } else if (e->object == registry && e->opcode == 0) {
        const char *iface = arg_string(e, 1);
        uint32_t name = e->args[0];
        if (!strcmp(iface, "wl_compositor"))
            global_name.compositor = name;
        else if (!strcmp(iface, "wl_shm"))
            global_name.shm = name;
        else if (!strcmp(iface, "xdg_wm_base"))
            global_name.wm_base = name;
        else if (!strcmp(iface, "zxdg_decoration_manager_v1"))
            global_name.decoration_manager = name;
        else if (!strcmp(iface, "wl_output"))
            global_name.output = name;
    } else if (e->object == sync_cb && e->opcode == 0) {
        sync_done = 1;
    } else if (e->object == shm && e->opcode == 0) {
        if (e->args[0] == FORMAT_ARGB8888)
            formats |= 1;
        if (e->args[0] == FORMAT_XRGB8888)
            formats |= 2;
    } else if (e->object == output && e->opcode == 1) {
        mode_width = (int32_t)e->args[1];
        mode_height = (int32_t)e->args[2];
    } else if (e->object == output && e->opcode == 3) {
        output_scale = (int32_t)e->args[0];
    } else if (e->object == wm_base && e->opcode == 0) {
        begin(); /* ping: pong */
        put(e->args[0]);
        end(&conn, wm_base, 3);
        flush(&conn);
    } else if (e->object == surface && e->opcode == 0) {
        entered = 1;
    } else if (e->object == surface && e->opcode == 2) {
        preferred_scale = e->args[0];
    } else if (e->object == xdg_surface && e->opcode == 0) {
        configure_serial = e->args[0];
        configured = 1;
    } else if (e->object == toplevel && e->opcode == 1) {
        closed = 1;
    } else if (e->object == decoration && e->opcode == 0) {
        decoration_mode = e->args[0];
    } else if (e->object == buffer && e->opcode == 0) {
        released = 1;
    } else if (e->object == frame && e->opcode == 0) {
        frame_done = 1;
    }
}

/* Send what is queued, then handle events until `*flag` is set. */
static int wait_for(int *flag) {
    struct event e;
    flush(&conn);
    while (!*flag && !protocol_error) {
        if (!next_event(&conn, &e))
            return 0;
        handle(&e);
    }
    return *flag;
}

static int roundtrip(void) {
    sync_cb = new_id(&conn);
    sync_done = 0;
    begin();
    put(sync_cb);
    end(&conn, 1, 0); /* wl_display.sync */
    return wait_for(&sync_done);
}

/* wl_registry.bind: name, then the new_id with its interface and version. */
static uint32_t bind_global(uint32_t name, const char *iface, uint32_t version) {
    uint32_t id = new_id(&conn);
    begin();
    put(name);
    put_string(iface);
    put(version);
    put(id);
    end(&conn, registry, 0);
    return id;
}

/* Ask the compositor for a picture of the output (SIGUSR1 to the
 * compositor that started us; it writes $POLY_COMPOSITOR_DUMP, a PPM,
 * renamed into place when complete) and count our window's pixels in it:
 * each half of the buffer at the output's scale. */
static void check_composed(void) {
    const char *path = getenv("POLY_COMPOSITOR_DUMP");
    char got[96];
    struct stat st;
    if (!path) {
        printf("wl-client: not started by poly-compositor --exec: no picture to check\n");
        return;
    }
    unlink(path);
    kill(getppid(), SIGUSR1);
    int waited = 0;
    while (stat(path, &st) < 0 && waited < 10000) {
        usleep(20000);
        waited += 20;
    }
    FILE *f = fopen(path, "rb");
    int w = 0, h = 0, max = 0;
    if (!f || fscanf(f, "P6 %d %d %d", &w, &h, &max) != 3 || fgetc(f) != '\n') {
        check(0, "the compositor wrote a picture of the output", f ? "not a PPM" : "no file");
        if (f)
            fclose(f);
        return;
    }
    snprintf(got, sizeof got, "%dx%d", w, h);
    check(w == mode_width && h == mode_height && max == 255, "the picture is the output's size", got);
    long left = 0, right = 0;
    unsigned char rgb[3];
    while (fread(rgb, 1, 3, f) == 3) {
        uint32_t p = (uint32_t)rgb[0] << 16 | (uint32_t)rgb[1] << 8 | rgb[2];
        left += p == LEFT;
        right += p == RIGHT;
    }
    fclose(f);
    long want = (long)(WIDTH / 2) * HEIGHT * output_scale * output_scale;
    snprintf(got, sizeof got, "%ld and %ld pixels, want %ld each", left, right, want);
    check(left == want && right == want, "the window's pixels are on the output", got);
}

/* A request to an object that does not exist is wl_display.error
 * invalid_object, on a connection of its own. */
static void check_unknown_object(void) {
    static struct conn bad;
    struct event e;
    if (connect_display(&bad) < 0)
        die("second connection");
    begin();
    end(&bad, 99, 0);
    flush(&bad);
    int got = next_event(&bad, &e);
    char why[64];
    snprintf(why, sizeof why, "object %u opcode %u", got ? e.object : 0, got ? e.opcode : 0);
    check(got && e.object == 1 && e.opcode == 0 && e.args[1] == ERROR_INVALID_OBJECT,
          "a request to an unknown object is an invalid_object error", why);
    close(bad.fd);
}

int main(void) {
    char got[96];
    if (connect_display(&conn) < 0)
        die("connect to the compositor");
    check(1, "connected to the compositor", "");

    registry = new_id(&conn);
    begin();
    put(registry);
    end(&conn, 1, 1); /* wl_display.get_registry */
    check(roundtrip(), "the registry's globals arrive before the sync callback", "no callback");
    int all = global_name.compositor && global_name.shm && global_name.wm_base && global_name.decoration_manager &&
              global_name.output;
    check(all, "wl_compositor, wl_shm, xdg_wm_base, zxdg_decoration_manager_v1 and wl_output are offered",
          "one is missing");
    if (!all)
        goto done;

    compositor = bind_global(global_name.compositor, "wl_compositor", 4);
    shm = bind_global(global_name.shm, "wl_shm", 1);
    wm_base = bind_global(global_name.wm_base, "xdg_wm_base", 1);
    decoration_manager = bind_global(global_name.decoration_manager, "zxdg_decoration_manager_v1", 1);
    output = bind_global(global_name.output, "wl_output", 2);
    roundtrip();
    snprintf(got, sizeof got, "formats %d", formats);
    check(formats == 3, "wl_shm offers argb8888 and xrgb8888", got);
    printf("wl-client: output %dx%d scale %d\n", mode_width, mode_height, output_scale);
    check(mode_width > 0 && mode_height > 0 && output_scale >= 1, "wl_output describes its mode and scale", "no mode");

    surface = new_id(&conn);
    begin();
    put(surface);
    end(&conn, compositor, 0); /* create_surface */
    xdg_surface = new_id(&conn);
    begin();
    put(xdg_surface);
    put(surface);
    end(&conn, wm_base, 2); /* get_xdg_surface */
    toplevel = new_id(&conn);
    begin();
    put(toplevel);
    end(&conn, xdg_surface, 1); /* get_toplevel */
    begin();
    put_string("wl-client");
    end(&conn, toplevel, 2); /* set_title */
    begin();
    put_string("poly.wl-client");
    end(&conn, toplevel, 3); /* set_app_id */
    decoration = new_id(&conn);
    begin();
    put(decoration);
    put(toplevel);
    end(&conn, decoration_manager, 1); /* get_toplevel_decoration */
    begin();
    put(MODE_SERVER_SIDE);
    end(&conn, decoration, 1); /* set_mode */
    begin();
    end(&conn, surface, 6); /* commit, no buffer */
    check(wait_for(&configured), "the initial commit is answered with xdg_surface.configure", "none");
    snprintf(got, sizeof got, "mode %u", decoration_mode);
    check(decoration_mode == MODE_SERVER_SIDE, "the decoration is server-side", got);
    begin();
    put(configure_serial);
    end(&conn, xdg_surface, 4); /* ack_configure */

    /* Shared memory: a file in /dev/shm, unlinked, sized, mapped. */
    char path[64];
    snprintf(path, sizeof path, "/dev/shm/wl-client-%d", (int)getpid());
    int memfd = open(path, O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
    if (memfd < 0)
        die("open a file in /dev/shm");
    unlink(path);
    size_t size = WIDTH * HEIGHT * 4;
    if (ftruncate(memfd, (off_t)size) < 0)
        die("ftruncate");
    uint32_t *pixels = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, memfd, 0);
    if (pixels == MAP_FAILED)
        die("mmap the buffer");
    for (int y = 0; y < HEIGHT; y++)
        for (int x = 0; x < WIDTH; x++)
            pixels[y * WIDTH + x] = x < WIDTH / 2 ? LEFT : RIGHT;
    check(1, "a shared memory buffer in /dev/shm", "");

    pool = new_id(&conn);
    begin();
    put(pool);
    put((uint32_t)size);
    end(&conn, shm, 0); /* create_pool: new_id, fd (out of band), size */
    conn.out_fd = memfd;
    flush(&conn);
    close(memfd);
    buffer = new_id(&conn);
    begin();
    put(buffer);
    put(0);
    put(WIDTH);
    put(HEIGHT);
    put(WIDTH * 4);
    put(FORMAT_XRGB8888);
    end(&conn, pool, 0); /* create_buffer */
    begin();
    end(&conn, pool, 1); /* the pool may go; the buffer keeps it */
    begin();
    put(buffer);
    put(0);
    put(0);
    end(&conn, surface, 1); /* attach */
    begin();
    put(0);
    put(0);
    put(WIDTH);
    put(HEIGHT);
    end(&conn, surface, 9); /* damage_buffer */
    frame = new_id(&conn);
    begin();
    put(frame);
    end(&conn, surface, 3); /* frame */
    begin();
    end(&conn, surface, 6); /* commit */
    check(wait_for(&frame_done), "the frame callback is done after the commit", "no callback");
    check(released || wait_for(&released), "the buffer is released", "no release");
    roundtrip();
    check(frame_deleted, "the frame callback's id is deleted", "no delete_id");
    check(entered, "the surface entered the output", "no enter");
    check(!protocol_error, "no protocol error", "an error");
    snprintf(got, sizeof got, "scale %u", preferred_scale);
    check(preferred_scale >= 1, "a preferred buffer scale", got);
    printf("wl-client: window %dx%d mapped, title \"wl-client\"\n", WIDTH, HEIGHT);
    check_composed();
    check_unknown_object();
    (void)closed;

done:
    printf("summary %d passed, %d failed\n", passed, failed);
    if (failed == 0)
        printf("wl-client: ok\n");
    fflush(stdout);
    close(conn.fd);
    return failed ? 1 : 0;
}
