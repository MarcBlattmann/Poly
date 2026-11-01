/*
 * wl-demo: a small interactive Wayland client for Poly's desktop (ADR 0107),
 * the "Poly Demo" app of the launcher. Like wl-client.c it speaks the wire
 * protocol itself, written from the protocol XML (wayland.xml, xdg-shell.xml,
 * xdg-decoration-unstable-v1.xml): no libwayland. Linked statically.
 *
 * It opens a 360x240 toplevel with a server-side decoration and stays until
 * its window is closed (the frame's close button) or Q is pressed. It binds
 * the seat and reports what reaches it, one line each, so a boot test can
 * check the compositor's input routing: the keymap (its file descriptor
 * mapped and checked to hold an XKB keymap), keyboard enter, keys,
 * modifiers, pointer enter, motion (the first only), buttons and the wheel.
 * A left click swaps the window's colour (DEMO_A <-> DEMO_B) and commits
 * again, which shows on the screen.
 *
 * Not in tests/linux/ itself: the diff runner compares those with real
 * Linux, which has no Poly compositor to talk to.
 */
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

#define WIDTH 360
#define HEIGHT 240
/* The window's two looks: a teal and an amber, with a lighter band. */
#define DEMO_A 0x2f7d6du
#define DEMO_B 0xc08a2eu
#define BAND 0xe8eef0u

#define FORMAT_XRGB8888 1
#define MODE_SERVER_SIDE 2
#define KEY_Q 16
#define BTN_LEFT 0x110

static void die(const char *what) {
    printf("wl-demo: %s failed (%s)\n", what, strerror(errno));
    exit(1);
}

/* ---- The connection ------------------------------------------------------ */

static int sock;
static uint8_t out[8192];
static size_t out_len;
static int out_fd = -1;
static uint8_t in[65536];
static size_t in_len;
static int in_fds[16];
static int in_nfds;
static uint32_t next_id = 2;

static uint32_t body[512];
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

static void end(uint32_t object, uint16_t opcode) {
    uint32_t size = 8 + 4 * (uint32_t)body_len;
    uint32_t header[2] = {object, size << 16 | opcode};
    memcpy(out + out_len, header, 8);
    memcpy(out + out_len + 8, body, 4 * body_len);
    out_len += size;
}

static void flush_out(void) {
    struct iovec iov = {.iov_base = out, .iov_len = out_len};
    union {
        struct cmsghdr align;
        char buf[CMSG_SPACE(sizeof(int))];
    } control;
    struct msghdr msg = {.msg_iov = &iov, .msg_iovlen = 1};
    if (out_fd >= 0) {
        msg.msg_control = control.buf;
        msg.msg_controllen = sizeof control.buf;
        struct cmsghdr *cmsg = CMSG_FIRSTHDR(&msg);
        cmsg->cmsg_level = SOL_SOCKET;
        cmsg->cmsg_type = SCM_RIGHTS;
        cmsg->cmsg_len = CMSG_LEN(sizeof(int));
        memcpy(CMSG_DATA(cmsg), &out_fd, sizeof(int));
    }
    while (iov.iov_len > 0) {
        ssize_t n = sendmsg(sock, &msg, MSG_NOSIGNAL);
        if (n < 0 && errno == EINTR)
            continue;
        if (n < 0)
            die("sendmsg");
        iov.iov_base = (char *)iov.iov_base + n;
        iov.iov_len -= (size_t)n;
        msg.msg_control = NULL;
        msg.msg_controllen = 0;
    }
    out_len = 0;
    out_fd = -1;
}

/* Read more bytes and any file descriptors; 0 at the end. */
static int fill(void) {
    struct iovec iov = {.iov_base = in + in_len, .iov_len = sizeof in - in_len};
    union {
        struct cmsghdr align;
        char buf[CMSG_SPACE(sizeof(int) * 8)];
    } control;
    struct msghdr msg = {.msg_iov = &iov, .msg_iovlen = 1, .msg_control = control.buf,
                         .msg_controllen = sizeof control.buf};
    ssize_t n = recvmsg(sock, &msg, MSG_CMSG_CLOEXEC);
    if (n < 0 && errno == EINTR)
        return 1;
    if (n <= 0)
        return 0;
    for (struct cmsghdr *c = CMSG_FIRSTHDR(&msg); c; c = CMSG_NXTHDR(&msg, c)) {
        if (c->cmsg_level != SOL_SOCKET || c->cmsg_type != SCM_RIGHTS)
            continue;
        int k = (int)((c->cmsg_len - CMSG_LEN(0)) / sizeof(int));
        for (int i = 0; i < k && in_nfds < 16; i++)
            memcpy(&in_fds[in_nfds++], CMSG_DATA(c) + i * sizeof(int), sizeof(int));
    }
    in_len += (size_t)n;
    return 1;
}

static int take_fd(void) {
    if (in_nfds == 0)
        return -1;
    int fd = in_fds[0];
    memmove(in_fds, in_fds + 1, sizeof(int) * (size_t)--in_nfds);
    return fd;
}

struct event {
    uint32_t object;
    uint16_t opcode;
    uint32_t args[256];
};

/* The next event (blocking); 0 when the compositor went away. */
static int next_event(struct event *e) {
    for (;;) {
        if (in_len >= 8) {
            uint32_t header[2];
            memcpy(header, in, 8);
            uint32_t size = header[1] >> 16;
            if (size >= 8 && size <= 8 + sizeof e->args && in_len >= size) {
                e->object = header[0];
                e->opcode = (uint16_t)header[1];
                memcpy(e->args, in + 8, size - 8);
                memmove(in, in + size, in_len - size);
                in_len -= size;
                return 1;
            }
        }
        if (!fill())
            return 0;
    }
}

static const char *arg_string(const struct event *e, uint32_t at) {
    return e->args[at] ? (const char *)&e->args[at + 1] : "";
}

/* ---- The client ---------------------------------------------------------- */

static uint32_t registry, compositor, shm, wm_base, decoration_manager, seat;
static uint32_t surface, xdg_surface, toplevel, decoration, buffer, pointer, keyboard, sync_cb;
static uint32_t name_compositor, name_shm, name_wm_base, name_decoration, name_seat;
static uint32_t *pixels;
static int configured, closed, sync_done, have_pointer, have_keyboard, motion_reported, colour_b;

static uint32_t bind_global(uint32_t name, const char *iface, uint32_t version) {
    uint32_t id = next_id++;
    begin();
    put(name);
    put_string(iface);
    put(version);
    put(id);
    end(registry, 0);
    return id;
}

static void draw(void) {
    uint32_t base = colour_b ? DEMO_B : DEMO_A;
    for (int y = 0; y < HEIGHT; y++)
        for (int x = 0; x < WIDTH; x++)
            pixels[y * WIDTH + x] = (y >= 100 && y < 140) ? BAND : base;
}

static void commit(void) {
    begin();
    put(buffer);
    put(0);
    put(0);
    end(surface, 1); /* attach */
    begin();
    put(0);
    put(0);
    put(WIDTH);
    put(HEIGHT);
    end(surface, 9); /* damage_buffer */
    begin();
    end(surface, 6); /* commit */
    flush_out();
}

/* The keymap: map it and check that it is an XKB keymap. */
static void keymap(uint32_t format, int fd, uint32_t size) {
    int ok = 0;
    if (fd >= 0 && size > 16) {
        char *map = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
        if (map != MAP_FAILED) {
            ok = !strncmp(map, "xkb_keymap {", 12) && map[size - 1] == '\0';
            munmap(map, size);
        }
    }
    if (fd >= 0)
        close(fd);
    printf("wl-demo: keymap format %u size %u %s\n", format, size, ok ? "xkb ok" : "unreadable");
}

static void handle(const struct event *e) {
    if (e->object == 1 && e->opcode == 0) {
        printf("wl-demo: protocol error on object %u, code %u: %s\n", e->args[0], e->args[1], arg_string(e, 2));
        exit(1);
    } else if (e->object == registry && e->opcode == 0) {
        const char *iface = arg_string(e, 1);
        if (!strcmp(iface, "wl_compositor"))
            name_compositor = e->args[0];
        else if (!strcmp(iface, "wl_shm"))
            name_shm = e->args[0];
        else if (!strcmp(iface, "xdg_wm_base"))
            name_wm_base = e->args[0];
        else if (!strcmp(iface, "zxdg_decoration_manager_v1"))
            name_decoration = e->args[0];
        else if (!strcmp(iface, "wl_seat"))
            name_seat = e->args[0];
    } else if (e->object == sync_cb && e->opcode == 0) {
        sync_done = 1;
    } else if (e->object == wm_base && e->opcode == 0) {
        begin();
        put(e->args[0]);
        end(wm_base, 3); /* pong */
        flush_out();
    } else if (e->object == xdg_surface && e->opcode == 0) {
        begin();
        put(e->args[0]);
        end(xdg_surface, 4); /* ack_configure */
        if (!configured)
            printf("wl-demo: configured\n");
        configured = 1;
        commit();
    } else if (e->object == toplevel && e->opcode == 1) {
        closed = 1;
    } else if (e->object == seat && e->opcode == 0) {
        uint32_t caps = e->args[0];
        if ((caps & 1) && !have_pointer) {
            pointer = next_id++;
            begin();
            put(pointer);
            end(seat, 0); /* get_pointer */
            have_pointer = 1;
        }
        if ((caps & 2) && !have_keyboard) {
            keyboard = next_id++;
            begin();
            put(keyboard);
            end(seat, 1); /* get_keyboard */
            have_keyboard = 1;
        }
        flush_out();
        printf("wl-demo: seat capabilities %u\n", caps);
    } else if (have_keyboard && e->object == keyboard) {
        switch (e->opcode) {
        case 0:
            keymap(e->args[0], take_fd(), e->args[1]);
            break;
        case 1:
            printf("wl-demo: keyboard enter\n");
            break;
        case 2:
            printf("wl-demo: keyboard leave\n");
            break;
        case 3:
            printf("wl-demo: key %u %s\n", e->args[2], e->args[3] ? "pressed" : "released");
            if (e->args[2] == KEY_Q && e->args[3])
                closed = 1;
            break;
        case 4:
            printf("wl-demo: modifiers depressed %u locked %u\n", e->args[1], e->args[3]);
            break;
        case 5:
            printf("wl-demo: repeat rate %u delay %u\n", e->args[0], e->args[1]);
            break;
        }
    } else if (have_pointer && e->object == pointer) {
        switch (e->opcode) {
        case 0:
            printf("wl-demo: pointer enter %d,%d\n", (int32_t)e->args[2] / 256, (int32_t)e->args[3] / 256);
            motion_reported = 0;
            break;
        case 1:
            printf("wl-demo: pointer leave\n");
            break;
        case 2:
            if (!motion_reported)
                printf("wl-demo: pointer motion %d,%d\n", (int32_t)e->args[1] / 256, (int32_t)e->args[2] / 256);
            motion_reported = 1;
            break;
        case 3:
            printf("wl-demo: button %u %s\n", e->args[2], e->args[3] ? "pressed" : "released");
            if (e->args[2] == BTN_LEFT && e->args[3]) {
                colour_b = !colour_b;
                draw();
                commit();
            }
            break;
        case 4:
            printf("wl-demo: axis %u %d\n", e->args[1], (int32_t)e->args[2] / 256);
            break;
        }
    }
    fflush(stdout);
}

static void roundtrip(void) {
    struct event e;
    sync_cb = next_id++;
    sync_done = 0;
    begin();
    put(sync_cb);
    end(1, 0); /* wl_display.sync */
    flush_out();
    while (!sync_done) {
        if (!next_event(&e))
            die("roundtrip");
        handle(&e);
    }
}

int main(void) {
    const char *dir = getenv("XDG_RUNTIME_DIR");
    const char *name = getenv("WAYLAND_DISPLAY");
    struct sockaddr_un addr = {.sun_family = AF_UNIX};
    if (!dir) {
        errno = ENOENT;
        die("XDG_RUNTIME_DIR");
    }
    snprintf(addr.sun_path, sizeof addr.sun_path, "%s/%s", dir, name ? name : "wayland-0");
    sock = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (sock < 0 || connect(sock, (struct sockaddr *)&addr, sizeof addr) < 0)
        die("connect");

    registry = next_id++;
    begin();
    put(registry);
    end(1, 1); /* get_registry */
    roundtrip();
    if (!name_compositor || !name_shm || !name_wm_base) {
        printf("wl-demo: the compositor lacks a global\n");
        return 1;
    }
    compositor = bind_global(name_compositor, "wl_compositor", 4);
    shm = bind_global(name_shm, "wl_shm", 1);
    wm_base = bind_global(name_wm_base, "xdg_wm_base", 1);
    if (name_decoration)
        decoration_manager = bind_global(name_decoration, "zxdg_decoration_manager_v1", 1);
    if (name_seat)
        seat = bind_global(name_seat, "wl_seat", 5);

    /* The buffer: a file in /dev/shm, unlinked, sized, mapped. */
    char path[64];
    snprintf(path, sizeof path, "/dev/shm/wl-demo-%d", (int)getpid());
    int memfd = open(path, O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
    if (memfd < 0)
        die("open a file in /dev/shm");
    unlink(path);
    size_t size = WIDTH * HEIGHT * 4;
    if (ftruncate(memfd, (off_t)size) < 0)
        die("ftruncate");
    pixels = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, memfd, 0);
    if (pixels == MAP_FAILED)
        die("mmap");
    draw();
    uint32_t pool = next_id++;
    begin();
    put(pool);
    put((uint32_t)size);
    end(shm, 0); /* create_pool */
    out_fd = memfd;
    flush_out();
    close(memfd);
    buffer = next_id++;
    begin();
    put(buffer);
    put(0);
    put(WIDTH);
    put(HEIGHT);
    put(WIDTH * 4);
    put(FORMAT_XRGB8888);
    end(pool, 0); /* create_buffer */
    begin();
    end(pool, 1); /* destroy the pool */

    surface = next_id++;
    begin();
    put(surface);
    end(compositor, 0); /* create_surface */
    xdg_surface = next_id++;
    begin();
    put(xdg_surface);
    put(surface);
    end(wm_base, 2); /* get_xdg_surface */
    toplevel = next_id++;
    begin();
    put(toplevel);
    end(xdg_surface, 1); /* get_toplevel */
    begin();
    put_string("Poly Demo");
    end(toplevel, 2); /* set_title */
    begin();
    put_string("poly.demo");
    end(toplevel, 3); /* set_app_id */
    if (decoration_manager) {
        decoration = next_id++;
        begin();
        put(decoration);
        put(toplevel);
        end(decoration_manager, 1); /* get_toplevel_decoration */
        begin();
        put(MODE_SERVER_SIDE);
        end(decoration, 1); /* set_mode */
    }
    begin();
    end(surface, 6); /* commit without a buffer: configure comes */
    flush_out();

    struct event e;
    while (!closed) {
        if (!next_event(&e)) {
            printf("wl-demo: the compositor went away\n");
            return 1;
        }
        handle(&e);
    }
    printf("wl-demo: closed\n");
    fflush(stdout);
    return 0;
}
