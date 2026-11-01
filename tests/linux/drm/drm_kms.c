/*
 * drm-kms: drive /dev/dri/card0 through the kernel's DRM/KMS ioctls alone,
 * with no libdrm: what a KMS client (a compositor, modetest, Mesa's GBM on
 * dumb buffers) does.
 *
 *  1. VERSION, GET_CAP, SET_CLIENT_CAP (universal planes, atomic), master.
 *  2. Enumerate: GETRESOURCES, GETCONNECTOR (the first connected one and
 *     its preferred mode), GETENCODER, GETCRTC, GETPLANERESOURCES,
 *     GETPLANE, OBJ_GETPROPERTIES / GETPROPERTY (the primary plane's
 *     "type", "FB_ID").
 *  3. Two dumb buffers (CREATE_DUMB, MAP_DUMB, mmap), pattern A drawn in
 *     the first, B in the second; ADDFB2 (XRGB8888) for both; SETCRTC with
 *     the first. Shows "A".
 *  4. PAGE_FLIP to the second with an event, poll(2) and read(2) of the
 *     flip-complete event. Shows "B".
 *  5. A yellow rectangle drawn into the shown buffer, DIRTYFB with its
 *     clip. Shows "C".
 *  6. WAIT_VBLANK (blocking, relative 1), and with an event.
 *  7. An atomic commit of FB_ID back to the first buffer, with an event,
 *     read without poll(2) (a blocking read).
 *  8. PRIME: HANDLE_TO_FD, lseek(SEEK_END) of the dma-buf, its mapping,
 *     FD_TO_HANDLE on the card (the same handle) and on renderD128, which
 *     refuses dumb buffers; a second card0 file is not master.
 *  9. RMFB, munmap, DESTROY_DUMB, close: the console gets the display back.
 *     Shows "closed".
 *
 * At each "drm-kms: show <name>" line the program waits for a line on its
 * standard input (the serial console), so that the boot test
 * (tools/xtask/src/drm.rs) can take a screenshot and compare it with the
 * pattern drawn on the host. Every failed check prints "drm-kms: FAIL ...";
 * the last line is "drm-kms: done, N failed" and the exit status 0 only if
 * N is 0. Structures and request numbers are the kernel's uapi headers
 * <drm/drm.h> and <drm/drm_mode.h> (MIT). Not part of the diff corpus (it
 * is in a subdirectory of tests/linux).
 */
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <drm/drm.h>
#include <drm/drm_fourcc.h>
#include <drm/drm_mode.h>

#ifndef DRM_PLANE_TYPE_PRIMARY
#define DRM_PLANE_TYPE_PRIMARY 1 /* the plane "type" property's Primary value */
#endif

#define CARD "/dev/dri/card0"
#define RENDER "/dev/dri/renderD128"

static int failed;

#define CHECK(cond, ...)                                       \
    do {                                                       \
        if (!(cond)) {                                         \
            printf("drm-kms: FAIL ");                          \
            printf(__VA_ARGS__);                               \
            printf(" (errno %d: %s)\n", errno, strerror(errno)); \
            failed++;                                          \
        }                                                      \
    } while (0)

#define U64(p) ((uint64_t)(uintptr_t)(p))

/* ioctl, restarted when a signal interrupted it (what libdrm's drmIoctl does). */
static int xioctl(int fd, unsigned long req, void *arg)
{
    int r;
    do {
        r = ioctl(fd, req, arg);
    } while (r == -1 && (errno == EINTR || errno == EAGAIN));
    return r;
}

/* The patterns; tools/xtask/src/drm.rs draws the same. */
static uint32_t pattern_a(uint32_t x, uint32_t y, uint32_t w, uint32_t h)
{
    uint32_t r = x * 255 / (w - 1), g = y * 255 / (h - 1), b = ((x / 64 + y / 64) & 1) ? 0xe0 : 0x20;
    return r << 16 | g << 8 | b;
}

static uint32_t pattern_b(uint32_t x, uint32_t y, uint32_t w, uint32_t h)
{
    (void)w;
    (void)h;
    return ((x / 32 + y / 32) & 1) ? 0x00c03030 : 0x003030c0;
}

/* The rectangle drawn for "C". */
#define RECT_X 100
#define RECT_Y 120
#define RECT_W 200
#define RECT_H 80
#define RECT_COLOR 0x00ffff00

static void draw(uint32_t *px, uint32_t pitch, uint32_t w, uint32_t h,
                 uint32_t (*f)(uint32_t, uint32_t, uint32_t, uint32_t))
{
    for (uint32_t y = 0; y < h; y++)
        for (uint32_t x = 0; x < w; x++)
            px[y * (pitch / 4) + x] = f(x, y, w, h);
}

/* Say what is shown and wait until the boot test has looked. */
static void show(const char *name)
{
    char line[64];
    printf("drm-kms: show %s\n", name);
    fflush(stdout);
    if (!fgets(line, sizeof line, stdin))
        printf("drm-kms: no answer on stdin after \"%s\"\n", name);
}

static uint64_t now_ns(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + ts.tv_nsec;
}

/* Read one event (blocking unless `wait_poll`, which polls first) and check it. */
static void expect_event(int fd, int wait_poll, uint32_t type, uint64_t user_data, uint32_t crtc,
                         unsigned *sequence, const char *what)
{
    if (wait_poll) {
        struct pollfd p = {.fd = fd, .events = POLLIN};
        int n = poll(&p, 1, 2000);
        CHECK(n == 1 && (p.revents & POLLIN), "%s: poll gave %d, revents %#x", what, n, p.revents);
    }
    char buf[1024];
    ssize_t n = read(fd, buf, sizeof buf);
    CHECK(n == (ssize_t)sizeof(struct drm_event_vblank), "%s: read %zd bytes", what, n);
    if (n < (ssize_t)sizeof(struct drm_event_vblank))
        return;
    struct drm_event_vblank e;
    memcpy(&e, buf, sizeof e);
    uint64_t stamp = (uint64_t)e.tv_sec * 1000000000ull + (uint64_t)e.tv_usec * 1000;
    uint64_t now = now_ns();
    CHECK(e.base.type == type && e.base.length == sizeof e, "%s: event type %u, length %u", what, e.base.type,
          e.base.length);
    CHECK(e.user_data == user_data, "%s: user_data %#llx", what, (unsigned long long)e.user_data);
    CHECK(e.crtc_id == crtc, "%s: crtc_id %u, expected %u", what, e.crtc_id, crtc);
    CHECK(stamp <= now && now - stamp < 2000000000ull, "%s: timestamp %llu, now %llu", what,
          (unsigned long long)stamp, (unsigned long long)now);
    if (sequence) {
        CHECK(e.sequence > *sequence || *sequence == 0, "%s: sequence %u after %u", what, e.sequence, *sequence);
        *sequence = e.sequence;
    }
}

/* The id of the property called `name` among `count` of `ids`, or 0. */
static uint32_t prop_named(int fd, const uint32_t *ids, uint32_t count, const char *name)
{
    for (uint32_t i = 0; i < count; i++) {
        struct drm_mode_get_property p = {.prop_id = ids[i]};
        if (xioctl(fd, DRM_IOCTL_MODE_GETPROPERTY, &p) == 0 && strcmp(p.name, name) == 0)
            return ids[i];
    }
    return 0;
}

struct dumb {
    uint32_t handle, pitch, fb;
    uint64_t size, offset;
    uint32_t *map;
};

static int make_dumb(int fd, uint32_t w, uint32_t h, struct dumb *d)
{
    struct drm_mode_create_dumb c = {.width = w, .height = h, .bpp = 32};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &c) == 0, "CREATE_DUMB %ux%u", w, h);
    CHECK(c.pitch >= w * 4 && c.size >= (uint64_t)c.pitch * h, "dumb pitch %u size %llu", c.pitch,
          (unsigned long long)c.size);
    struct drm_mode_map_dumb m = {.handle = c.handle};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &m) == 0, "MAP_DUMB");
    void *p = mmap(NULL, c.size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, m.offset);
    CHECK(p != MAP_FAILED, "mmap of the dumb buffer at %#llx", (unsigned long long)m.offset);
    if (p == MAP_FAILED)
        return -1;
    *d = (struct dumb){.handle = c.handle, .pitch = c.pitch, .size = c.size, .offset = m.offset, .map = p};
    struct drm_mode_fb_cmd2 f = {.width = w, .height = h, .pixel_format = DRM_FORMAT_XRGB8888};
    f.handles[0] = c.handle;
    f.pitches[0] = c.pitch;
    CHECK(xioctl(fd, DRM_IOCTL_MODE_ADDFB2, &f) == 0 && f.fb_id != 0, "ADDFB2");
    d->fb = f.fb_id;
    return 0;
}

int main(void)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    int fd = open(CARD, O_RDWR | O_CLOEXEC);
    CHECK(fd >= 0, "open " CARD);
    if (fd < 0)
        goto out;

    /* 1. Version, caps, master. */
    struct drm_version v = {0};
    CHECK(xioctl(fd, DRM_IOCTL_VERSION, &v) == 0, "VERSION");
    char name[64] = {0}, desc[128] = {0}, date[32] = {0};
    v.name = name;
    v.name_len = sizeof name - 1;
    v.desc = desc;
    v.desc_len = sizeof desc - 1;
    v.date = date;
    v.date_len = sizeof date - 1;
    CHECK(xioctl(fd, DRM_IOCTL_VERSION, &v) == 0, "VERSION with buffers");
    printf("drm-kms: driver %s %d.%d.%d (%s)\n", name, v.version_major, v.version_minor, v.version_patchlevel, desc);
    CHECK(strcmp(name, "virtio_gpu") == 0, "driver name %s", name);
    struct { uint64_t cap, want; } caps[] = {
        {DRM_CAP_DUMB_BUFFER, 1},
        {DRM_CAP_PRIME, DRM_PRIME_CAP_IMPORT | DRM_PRIME_CAP_EXPORT},
        {DRM_CAP_TIMESTAMP_MONOTONIC, 1},
        {DRM_CAP_CRTC_IN_VBLANK_EVENT, 1},
        {DRM_CAP_ADDFB2_MODIFIERS, 1},
    };
    for (size_t i = 0; i < sizeof caps / sizeof caps[0]; i++) {
        struct drm_get_cap c = {.capability = caps[i].cap};
        CHECK(xioctl(fd, DRM_IOCTL_GET_CAP, &c) == 0 && c.value == caps[i].want, "GET_CAP %llu = %llu",
              (unsigned long long)caps[i].cap, (unsigned long long)c.value);
    }
    struct drm_set_client_cap cc = {.capability = DRM_CLIENT_CAP_UNIVERSAL_PLANES, .value = 1};
    CHECK(xioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, &cc) == 0, "SET_CLIENT_CAP universal planes");
    cc = (struct drm_set_client_cap){.capability = DRM_CLIENT_CAP_ATOMIC, .value = 1};
    CHECK(xioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, &cc) == 0, "SET_CLIENT_CAP atomic");
    CHECK(xioctl(fd, DRM_IOCTL_SET_MASTER, NULL) == 0, "SET_MASTER (the first opener is master)");

    /* 2. Enumerate. */
    struct drm_mode_card_res res = {0};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &res) == 0, "GETRESOURCES");
    uint32_t crtcs[16] = {0}, conns[16] = {0}, encs[16] = {0};
    CHECK(res.count_crtcs >= 1 && res.count_crtcs <= 16 && res.count_connectors >= 1 &&
              res.count_connectors <= 16 && res.count_encoders <= 16,
          "%u CRTCs, %u connectors, %u encoders", res.count_crtcs, res.count_connectors, res.count_encoders);
    res.crtc_id_ptr = U64(crtcs);
    res.connector_id_ptr = U64(conns);
    res.encoder_id_ptr = U64(encs);
    res.count_fbs = 0;
    CHECK(xioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &res) == 0, "GETRESOURCES with arrays");
    printf("drm-kms: %u CRTC(s), %u connector(s), %u encoder(s), %ux%u to %ux%u\n", res.count_crtcs,
           res.count_connectors, res.count_encoders, res.min_width, res.min_height, res.max_width, res.max_height);

    uint32_t conn = 0, enc_id = 0;
    struct drm_mode_modeinfo mode = {0};
    for (uint32_t i = 0; i < res.count_connectors && !conn; i++) {
        struct drm_mode_get_connector c = {.connector_id = conns[i]};
        CHECK(xioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &c) == 0, "GETCONNECTOR %u", conns[i]);
        if (c.connection != 1 || c.count_modes == 0)
            continue;
        struct drm_mode_modeinfo modes[32];
        uint32_t ce[4];
        uint32_t props[16];
        uint64_t values[16];
        if (c.count_modes > 32 || c.count_encoders > 4 || c.count_props > 16)
            continue;
        c.modes_ptr = U64(modes);
        c.encoders_ptr = U64(ce);
        c.props_ptr = U64(props);
        c.prop_values_ptr = U64(values);
        CHECK(xioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &c) == 0, "GETCONNECTOR %u with arrays", conns[i]);
        conn = conns[i];
        mode = modes[0];
        enc_id = ce[0];
        CHECK(modes[0].type & DRM_MODE_TYPE_PREFERRED, "the first mode is not the preferred one");
        CHECK(prop_named(fd, props, c.count_props, "CRTC_ID") != 0, "the connector has no CRTC_ID");
        printf("drm-kms: connector %u (type %u-%u): %u modes, preferred %s %u Hz, %ux%u mm\n", conn,
               c.connector_type, c.connector_type_id, c.count_modes, modes[0].name, modes[0].vrefresh, c.mm_width,
               c.mm_height);
    }
    CHECK(conn != 0, "no connected connector");
    if (!conn)
        goto out;
    struct drm_mode_get_encoder enc = {.encoder_id = enc_id};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_GETENCODER, &enc) == 0 && enc.possible_crtcs != 0, "GETENCODER");
    uint32_t crtc = 0, crtc_index = 0;
    for (uint32_t i = 0; i < res.count_crtcs; i++)
        if (enc.possible_crtcs & (1u << i)) {
            crtc = crtcs[i];
            crtc_index = i;
            break;
        }
    struct drm_mode_crtc gc = {.crtc_id = crtc};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_GETCRTC, &gc) == 0, "GETCRTC %u", crtc);

    struct drm_mode_get_plane_res pr = {0};
    uint32_t planes[16];
    CHECK(xioctl(fd, DRM_IOCTL_MODE_GETPLANERESOURCES, &pr) == 0 && pr.count_planes >= 1 && pr.count_planes <= 16,
          "GETPLANERESOURCES: %u planes", pr.count_planes);
    pr.plane_id_ptr = U64(planes);
    CHECK(xioctl(fd, DRM_IOCTL_MODE_GETPLANERESOURCES, &pr) == 0, "GETPLANERESOURCES with the array");
    uint32_t plane = 0, fb_id_prop = 0;
    for (uint32_t i = 0; i < pr.count_planes && !plane; i++) {
        uint32_t formats[16];
        struct drm_mode_get_plane gp = {.plane_id = planes[i], .format_type_ptr = U64(formats), .count_format_types = 16};
        CHECK(xioctl(fd, DRM_IOCTL_MODE_GETPLANE, &gp) == 0, "GETPLANE %u", planes[i]);
        if (!(gp.possible_crtcs & (1u << crtc_index)))
            continue;
        int xrgb = 0;
        for (uint32_t f = 0; f < gp.count_format_types && f < 16; f++)
            xrgb |= formats[f] == DRM_FORMAT_XRGB8888;
        CHECK(xrgb, "plane %u lacks XRGB8888", planes[i]);
        uint32_t ids[32];
        uint64_t vals[32];
        struct drm_mode_obj_get_properties op = {
            .props_ptr = U64(ids), .prop_values_ptr = U64(vals), .count_props = 32,
            .obj_id = planes[i], .obj_type = DRM_MODE_OBJECT_PLANE};
        CHECK(xioctl(fd, DRM_IOCTL_MODE_OBJ_GETPROPERTIES, &op) == 0, "OBJ_GETPROPERTIES of plane %u", planes[i]);
        uint32_t type = prop_named(fd, ids, op.count_props, "type");
        for (uint32_t k = 0; k < op.count_props; k++)
            if (ids[k] == type && vals[k] == DRM_PLANE_TYPE_PRIMARY)
                plane = planes[i];
        fb_id_prop = prop_named(fd, ids, op.count_props, "FB_ID");
    }
    CHECK(plane != 0 && fb_id_prop != 0, "no primary plane with FB_ID for CRTC %u", crtc);

    /* 3. Dumb buffers, framebuffers, SETCRTC. */
    uint32_t w = mode.hdisplay, h = mode.vdisplay;
    struct dumb a, b;
    if (make_dumb(fd, w, h, &a) || make_dumb(fd, w, h, &b))
        goto out;
    draw(a.map, a.pitch, w, h, pattern_a);
    draw(b.map, b.pitch, w, h, pattern_b);
    struct drm_mode_crtc sc = {.crtc_id = crtc, .fb_id = a.fb, .set_connectors_ptr = U64(&conn),
                               .count_connectors = 1, .mode_valid = 1, .mode = mode};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_SETCRTC, &sc) == 0, "SETCRTC %ux%u", w, h);
    gc = (struct drm_mode_crtc){.crtc_id = crtc};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_GETCRTC, &gc) == 0 && gc.fb_id == a.fb && gc.mode_valid &&
              gc.mode.hdisplay == w,
          "GETCRTC after SETCRTC: fb %u, mode_valid %u", gc.fb_id, gc.mode_valid);
    show("A");

    /* 4. A page flip with an event. */
    unsigned seq = 0;
    struct drm_mode_crtc_page_flip flip = {.crtc_id = crtc, .fb_id = b.fb, .flags = DRM_MODE_PAGE_FLIP_EVENT,
                                           .user_data = 0xb0b0};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_PAGE_FLIP, &flip) == 0, "PAGE_FLIP");
    struct drm_mode_crtc_page_flip again = flip;
    CHECK(ioctl(fd, DRM_IOCTL_MODE_PAGE_FLIP, &again) == -1 && errno == EBUSY, "a second flip while one is pending");
    expect_event(fd, 1, DRM_EVENT_FLIP_COMPLETE, 0xb0b0, crtc, &seq, "flip");
    show("B");

    /* 5. Front-buffer drawing and DIRTYFB. */
    for (uint32_t y = RECT_Y; y < RECT_Y + RECT_H; y++)
        for (uint32_t x = RECT_X; x < RECT_X + RECT_W; x++)
            b.map[y * (b.pitch / 4) + x] = RECT_COLOR;
    struct drm_clip_rect clip = {RECT_X, RECT_Y, RECT_X + RECT_W, RECT_Y + RECT_H};
    struct drm_mode_fb_dirty_cmd dirty = {.fb_id = b.fb, .num_clips = 1, .clips_ptr = U64(&clip)};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_DIRTYFB, &dirty) == 0, "DIRTYFB");
    show("C");

    /* 6. Vblank waits. */
    union drm_wait_vblank wv = {.request = {.type = _DRM_VBLANK_RELATIVE, .sequence = 1}};
    CHECK(xioctl(fd, DRM_IOCTL_WAIT_VBLANK, &wv) == 0, "WAIT_VBLANK relative 1");
    CHECK(wv.reply.sequence > seq, "WAIT_VBLANK came back at vblank %u, the flip was at %u", wv.reply.sequence, seq);
    seq = wv.reply.sequence;
    wv = (union drm_wait_vblank){.request = {.type = _DRM_VBLANK_RELATIVE | _DRM_VBLANK_EVENT, .sequence = 2,
                                             .signal = 0x5151}};
    CHECK(xioctl(fd, DRM_IOCTL_WAIT_VBLANK, &wv) == 0, "WAIT_VBLANK with an event");
    expect_event(fd, 1, DRM_EVENT_VBLANK, 0x5151, crtc, &seq, "vblank event");

    /* 7. An atomic flip back, event read with a blocking read. */
    uint32_t objs[] = {plane}, counts[] = {1}, props[] = {fb_id_prop};
    uint64_t values[] = {a.fb};
    struct drm_mode_atomic at = {.flags = DRM_MODE_PAGE_FLIP_EVENT | DRM_MODE_ATOMIC_NONBLOCK, .count_objs = 1,
                                 .objs_ptr = U64(objs), .count_props_ptr = U64(counts), .props_ptr = U64(props),
                                 .prop_values_ptr = U64(values), .user_data = 0xa7a7};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_ATOMIC, &at) == 0, "MODE_ATOMIC flip");
    expect_event(fd, 0, DRM_EVENT_FLIP_COMPLETE, 0xa7a7, crtc, &seq, "atomic flip");
    at.flags = DRM_MODE_ATOMIC_TEST_ONLY;
    values[0] = 0x7fffffff;
    CHECK(ioctl(fd, DRM_IOCTL_MODE_ATOMIC, &at) == -1 && errno == ENOENT, "an atomic test with an unknown FB_ID");

    /* 8. PRIME. */
    struct drm_prime_handle ph = {.handle = a.handle, .flags = DRM_CLOEXEC | DRM_RDWR, .fd = -1};
    CHECK(xioctl(fd, DRM_IOCTL_PRIME_HANDLE_TO_FD, &ph) == 0 && ph.fd >= 0, "PRIME_HANDLE_TO_FD");
    if (ph.fd >= 0) {
        off_t size = lseek(ph.fd, 0, SEEK_END);
        CHECK(size == (off_t)a.size, "lseek(SEEK_END) of the dma-buf: %lld", (long long)size);
        uint32_t *p = mmap(NULL, a.size, PROT_READ, MAP_SHARED, ph.fd, 0);
        CHECK(p != MAP_FAILED, "mmap of the dma-buf");
        if (p != MAP_FAILED) {
            CHECK(p[0] == a.map[0] && p[(h - 1) * (a.pitch / 4) + w - 1] == a.map[(h - 1) * (a.pitch / 4) + w - 1],
                  "the dma-buf does not show the buffer");
            munmap(p, a.size);
        }
        struct drm_prime_handle back = {.fd = ph.fd};
        CHECK(xioctl(fd, DRM_IOCTL_PRIME_FD_TO_HANDLE, &back) == 0 && back.handle == a.handle,
              "PRIME_FD_TO_HANDLE on the card: handle %u, expected %u", back.handle, a.handle);
        int r = open(RENDER, O_RDWR | O_CLOEXEC);
        CHECK(r >= 0, "open " RENDER);
        if (r >= 0) {
            struct drm_prime_handle imp = {.fd = ph.fd};
            CHECK(xioctl(r, DRM_IOCTL_PRIME_FD_TO_HANDLE, &imp) == 0 && imp.handle != 0, "PRIME_FD_TO_HANDLE on %s",
                  RENDER);
            struct drm_mode_create_dumb c = {.width = 8, .height = 8, .bpp = 32};
            CHECK(ioctl(r, DRM_IOCTL_MODE_CREATE_DUMB, &c) == -1 && errno == EACCES, "CREATE_DUMB on a render node");
            struct drm_gem_close gcl = {.handle = imp.handle};
            CHECK(xioctl(r, DRM_IOCTL_GEM_CLOSE, &gcl) == 0, "GEM_CLOSE on the render node");
            close(r);
        }
        close(ph.fd);
    }
    int second = open(CARD, O_RDWR | O_CLOEXEC);
    CHECK(second >= 0, "a second open of " CARD);
    if (second >= 0) {
        CHECK(ioctl(second, DRM_IOCTL_MODE_SETCRTC, &sc) == -1 && errno == EACCES, "SETCRTC by a file that is not master");
        close(second);
    }

    /* 9. Clean up; the console comes back when the master closes. */
    CHECK(xioctl(fd, DRM_IOCTL_MODE_RMFB, &a.fb) == 0, "RMFB");
    CHECK(xioctl(fd, DRM_IOCTL_MODE_RMFB, &b.fb) == 0, "RMFB");
    munmap(a.map, a.size);
    munmap(b.map, b.size);
    struct drm_mode_destroy_dumb da = {.handle = a.handle}, db = {.handle = b.handle};
    CHECK(xioctl(fd, DRM_IOCTL_MODE_DESTROY_DUMB, &da) == 0, "DESTROY_DUMB");
    CHECK(xioctl(fd, DRM_IOCTL_MODE_DESTROY_DUMB, &db) == 0, "DESTROY_DUMB");
    close(fd);
    show("closed");
out:
    printf("drm-kms: done, %d failed\n", failed);
    return failed != 0;
}
