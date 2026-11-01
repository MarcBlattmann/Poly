/*
 * pcm-tone: play sine tones on ALSA card 0, PCM device 0 (what
 * `aplay -D hw:0,0` opens) through the kernel's ALSA ioctls alone, with no
 * alsa-lib: the control device's version, card and PCM queries, then per
 * tone HW_REFINE, HW_PARAMS, SW_PARAMS, PREPARE, frames through
 * WRITEI_FRAMES (or write(2)) under poll(2), STATUS, SYNC_PTR and DRAIN;
 * then an underrun (XRUN, EPIPE, recovery by PREPARE) and DROP.
 *
 * Arguments: tones as RATE:HZ:MS[:[w][:FORMAT]] ("w" writes with write(2)
 * instead of the ioctl; FORMAT is s16 (the default), u8, s32 or float, all
 * stereo at half of full scale); default "48000:440:1500 44100:1000:1500:w".
 * Further modes (ADR 0109):
 *   mmap:RATE:HZ:MS:VOL   set "Master Playback Volume" to VOL (of 100)
 *                         through the control device, then play the tone
 *                         with mmap access: the ring buffer, status and
 *                         control pages mapped, the application pointer
 *                         moved in the control page for the first half
 *                         and through SYNC_PTR for the second;
 *   mix:HZ1:HZ2:MS        two processes play at once (the parent HZ1 at
 *                         48 kHz, a child HZ2 at 44.1 kHz), each at a
 *                         quarter of full scale, through two subdevices;
 *   capture:RATE:MS       record from /dev/snd/pcmC0D0c through
 *                         READI_FRAMES and read(2), check that it takes
 *                         real time, report the level, then an overrun.
 * Every check prints
 * "pcm-tone: FAIL ..." when it fails; the last line is
 * "pcm-tone: done, N failed" and the exit status 0 only if N is 0.
 *
 * The structures and request numbers come from the kernel's uapi header
 * <sound/asound.h>; the boot test (tools/xtask/src/sound.rs) records what
 * QEMU's virtio-sound device played and checks its frequency and level.
 * Not part of the diff corpus (it is in a subdirectory of tests/linux).
 */
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>
#include <sound/asound.h>

#include <sys/mman.h>
#include <sys/wait.h>

#define CTL_PATH "/dev/snd/controlC0"
#define PCM_PATH "/dev/snd/pcmC0D0p"
#define CAPTURE_PATH "/dev/snd/pcmC0D0c"
#define CHANNELS 2
#define PERIOD 1024
#define PERIODS 4
#define SUBDEVICES 8

/* The tones' amplitude: half of full scale, a quarter for mixed ones. */
static double amplitude = 16384.0;

static int failed;

/* The sample formats a tone may use. */
struct format {
    const char *name;
    snd_pcm_format_t alsa;
    int bytes;
    unsigned msbits;
};

static const struct format FORMATS[] = {
    {"s16", SNDRV_PCM_FORMAT_S16_LE, 2, 16},
    {"u8", SNDRV_PCM_FORMAT_U8, 1, 8},
    {"s32", SNDRV_PCM_FORMAT_S32_LE, 4, 32},
    {"float", SNDRV_PCM_FORMAT_FLOAT_LE, 4, 32},
};

/* Store sample value `v` (-1..1) in format `f` at `out`. */
static void put_sample(const struct format *f, unsigned char *out, double v)
{
    if (f->alsa == SNDRV_PCM_FORMAT_U8) {
        out[0] = (unsigned char)lrint(128 + 127 * v);
    } else if (f->alsa == SNDRV_PCM_FORMAT_S32_LE) {
        int32_t s = (int32_t)lrint(2147483647.0 * v);
        memcpy(out, &s, 4);
    } else if (f->alsa == SNDRV_PCM_FORMAT_FLOAT_LE) {
        float s = (float)v;
        memcpy(out, &s, 4);
    } else {
        int16_t s = (int16_t)lrint(32767.0 * v);
        memcpy(out, &s, 2);
    }
}

#define CHECK(cond, ...)                         \
    do {                                         \
        if (!(cond)) {                           \
            printf("pcm-tone: FAIL ");           \
            printf(__VA_ARGS__);                 \
            printf(" (%s)\n", strerror(errno));  \
            failed++;                            \
        }                                        \
    } while (0)

static double now_s(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}

static struct snd_mask *mask(struct snd_pcm_hw_params *p, int n)
{
    return &p->masks[n - SNDRV_PCM_HW_PARAM_FIRST_MASK];
}

static struct snd_interval *interval(struct snd_pcm_hw_params *p, int n)
{
    return &p->intervals[n - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL];
}

static int mask_test(struct snd_mask *m, unsigned v)
{
    return (m->bits[v / 32] >> (v % 32)) & 1;
}

static void mask_only(struct snd_mask *m, unsigned v)
{
    memset(m, 0, sizeof(*m));
    m->bits[v / 32] = 1u << (v % 32);
}

/* What snd_pcm_hw_params_any starts from: everything allowed. */
static void params_any(struct snd_pcm_hw_params *p)
{
    memset(p, 0, sizeof(*p));
    for (int n = SNDRV_PCM_HW_PARAM_FIRST_MASK; n <= SNDRV_PCM_HW_PARAM_LAST_MASK; n++)
        memset(mask(p, n), 0xff, sizeof(struct snd_mask));
    for (int n = SNDRV_PCM_HW_PARAM_FIRST_INTERVAL; n <= SNDRV_PCM_HW_PARAM_LAST_INTERVAL; n++) {
        interval(p, n)->min = 0;
        interval(p, n)->max = ~0u;
    }
    p->rmask = ~0u;
}

static void set_single(struct snd_pcm_hw_params *p, int n, unsigned v)
{
    struct snd_interval *i = interval(p, n);
    i->min = i->max = v;
    i->openmin = i->openmax = 0;
    i->integer = 1;
}

static void control_checks(void)
{
    int ctl = open(CTL_PATH, O_RDWR);
    CHECK(ctl >= 0, "open " CTL_PATH);
    if (ctl < 0)
        return;
    int version = 0;
    CHECK(ioctl(ctl, SNDRV_CTL_IOCTL_PVERSION, &version) == 0 && version >= SNDRV_PROTOCOL_VERSION(2, 0, 0),
          "SNDRV_CTL_IOCTL_PVERSION gave %#x", version);
    struct snd_ctl_card_info card;
    memset(&card, 0, sizeof(card));
    CHECK(ioctl(ctl, SNDRV_CTL_IOCTL_CARD_INFO, &card) == 0 && card.card == 0, "SNDRV_CTL_IOCTL_CARD_INFO");
    printf("pcm-tone: card %d: id %s, driver %s, %s\n", card.card, (char *)card.id, (char *)card.driver,
           (char *)card.longname);
    int dev = -1;
    CHECK(ioctl(ctl, SNDRV_CTL_IOCTL_PCM_NEXT_DEVICE, &dev) == 0 && dev == 0, "first PCM device is %d", dev);
    struct snd_pcm_info info;
    memset(&info, 0, sizeof(info));
    info.device = 0;
    info.subdevice = 0;
    info.stream = SNDRV_PCM_STREAM_PLAYBACK;
    CHECK(ioctl(ctl, SNDRV_CTL_IOCTL_PCM_INFO, &info) == 0, "SNDRV_CTL_IOCTL_PCM_INFO for device 0");
    printf("pcm-tone: pcm 0: %s, %u subdevice(s)\n", (char *)info.name, info.subdevices_count);
    info.device = 7;
    CHECK(ioctl(ctl, SNDRV_CTL_IOCTL_PCM_INFO, &info) < 0 && errno == ENXIO, "PCM_INFO of a missing device");
    close(ctl);
}

/* Set the stream up for format `f`, stereo, at `rate`, PERIOD frames x
 * PERIODS. */
static int setup_access(int fd, const struct format *f, unsigned rate, unsigned long start_threshold, unsigned access)
{
    struct snd_pcm_hw_params p;
    params_any(&p);
    if (ioctl(fd, SNDRV_PCM_IOCTL_HW_REFINE, &p) != 0) {
        CHECK(0, "HW_REFINE of everything");
        return -1;
    }
    CHECK(mask_test(mask(&p, SNDRV_PCM_HW_PARAM_FORMAT), f->alsa), "%s is offered", f->name);
    CHECK(mask_test(mask(&p, SNDRV_PCM_HW_PARAM_ACCESS), SNDRV_PCM_ACCESS_RW_INTERLEAVED), "RW_INTERLEAVED");
    struct snd_interval *r = interval(&p, SNDRV_PCM_HW_PARAM_RATE);
    struct snd_interval *c = interval(&p, SNDRV_PCM_HW_PARAM_CHANNELS);
    CHECK(r->min <= rate && rate <= r->max, "rate %u in %u..%u", rate, r->min, r->max);
    CHECK(c->min <= CHANNELS && CHANNELS <= c->max, "stereo in %u..%u", c->min, c->max);
    static int shown;
    if (!shown++)
        printf("pcm-tone: refined: rates %u-%u, channels %u-%u, period bytes %u-%u, periods %u-%u, buffer bytes <= %u\n",
               r->min, r->max, c->min, c->max, interval(&p, SNDRV_PCM_HW_PARAM_PERIOD_BYTES)->min,
               interval(&p, SNDRV_PCM_HW_PARAM_PERIOD_BYTES)->max, interval(&p, SNDRV_PCM_HW_PARAM_PERIODS)->min,
               interval(&p, SNDRV_PCM_HW_PARAM_PERIODS)->max, interval(&p, SNDRV_PCM_HW_PARAM_BUFFER_BYTES)->max);

    CHECK(mask_test(mask(&p, SNDRV_PCM_HW_PARAM_ACCESS), access), "access %u is offered", access);
    params_any(&p);
    mask_only(mask(&p, SNDRV_PCM_HW_PARAM_ACCESS), access);
    mask_only(mask(&p, SNDRV_PCM_HW_PARAM_FORMAT), f->alsa);
    mask_only(mask(&p, SNDRV_PCM_HW_PARAM_SUBFORMAT), SNDRV_PCM_SUBFORMAT_STD);
    set_single(&p, SNDRV_PCM_HW_PARAM_CHANNELS, CHANNELS);
    set_single(&p, SNDRV_PCM_HW_PARAM_RATE, rate);
    set_single(&p, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, PERIOD);
    set_single(&p, SNDRV_PCM_HW_PARAM_PERIODS, PERIODS);
    if (ioctl(fd, SNDRV_PCM_IOCTL_HW_PARAMS, &p) != 0) {
        CHECK(0, "HW_PARAMS %s stereo %u Hz", f->name, rate);
        return -1;
    }
    unsigned buffer = interval(&p, SNDRV_PCM_HW_PARAM_BUFFER_SIZE)->min;
    CHECK(buffer == PERIOD * PERIODS && interval(&p, SNDRV_PCM_HW_PARAM_BUFFER_SIZE)->max == buffer,
          "buffer of %u frames", buffer);
    CHECK(p.rate_num == rate && p.rate_den == 1 && p.msbits == f->msbits, "rate %u/%u, msbits %u", p.rate_num,
          p.rate_den, p.msbits);

    struct snd_pcm_sw_params sw;
    memset(&sw, 0, sizeof(sw));
    sw.period_step = 1;
    sw.avail_min = PERIOD;
    sw.start_threshold = start_threshold;
    sw.stop_threshold = PERIOD * PERIODS;
    sw.proto = SNDRV_PCM_VERSION;
    sw.tstamp_type = SNDRV_PCM_TSTAMP_TYPE_MONOTONIC;
    if (ioctl(fd, SNDRV_PCM_IOCTL_SW_PARAMS, &sw) != 0) {
        CHECK(0, "SW_PARAMS");
        return -1;
    }
    CHECK(sw.boundary >= PERIOD * PERIODS && sw.boundary % (PERIOD * PERIODS) == 0, "boundary %lu", sw.boundary);
    if (ioctl(fd, SNDRV_PCM_IOCTL_PREPARE) != 0) {
        CHECK(0, "PREPARE");
        return -1;
    }
    struct snd_pcm_status st;
    memset(&st, 0, sizeof(st));
    int capture = 0;
    struct snd_pcm_info info;
    memset(&info, 0, sizeof(info));
    if (ioctl(fd, SNDRV_PCM_IOCTL_INFO, &info) == 0)
        capture = info.stream == SNDRV_PCM_STREAM_CAPTURE;
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_STATUS, &st) == 0 && st.state == SNDRV_PCM_STATE_PREPARED &&
              st.avail == (capture ? 0 : PERIOD * PERIODS),
          "prepared with an empty buffer (state %d, avail %lu)", st.state, st.avail);
    return 0;
}

static int setup(int fd, const struct format *f, unsigned rate, unsigned long start_threshold)
{
    return setup_access(fd, f, rate, start_threshold, SNDRV_PCM_ACCESS_RW_INTERLEAVED);
}

/* Write `frames` frames of `fb` bytes, waiting with poll(2) for room each
 * time. */
static long write_frames(int fd, const unsigned char *buf, long frames, int fb, int use_write)
{
    long done = 0;
    while (done < frames) {
        struct pollfd pfd = {.fd = fd, .events = POLLOUT};
        int n = poll(&pfd, 1, 5000);
        if (n != 1 || !(pfd.revents & POLLOUT) || (pfd.revents & POLLERR)) {
            CHECK(0, "poll for room: %d, revents %#x", n, pfd.revents);
            return done;
        }
        long chunk = frames - done < PERIOD ? frames - done : PERIOD;
        long got;
        if (use_write) {
            ssize_t w = write(fd, buf + done * fb, chunk * fb);
            got = w < 0 ? -1 : w / fb;
        } else {
            struct snd_xferi x = {.result = 0, .buf = (void *)(buf + done * fb), .frames = chunk};
            got = ioctl(fd, SNDRV_PCM_IOCTL_WRITEI_FRAMES, &x) == 0 ? x.result : -1;
        }
        if (got <= 0) {
            CHECK(0, "%s of %ld frames gave %ld", use_write ? "write" : "WRITEI_FRAMES", chunk, got);
            return done;
        }
        done += got;
    }
    return done;
}

static void play(int fd, const struct format *f, unsigned rate, unsigned hz, unsigned ms, int use_write)
{
    long frames = (long)rate * ms / 1000;
    int fb = f->bytes * CHANNELS;
    unsigned char *buf = malloc(frames * fb);
    if (!buf) {
        CHECK(0, "malloc");
        return;
    }
    for (long i = 0; i < frames; i++) {
        double v = amplitude / 32768.0 * sin(2 * M_PI * hz * (double)i / rate);
        for (int c = 0; c < CHANNELS; c++)
            put_sample(f, buf + i * fb + c * f->bytes, v);
    }
    if (setup(fd, f, rate, PERIOD * PERIODS) != 0) {
        free(buf);
        return;
    }
    double t0 = now_s();
    long done = write_frames(fd, buf, frames, fb, use_write);
    CHECK(done == frames, "wrote %ld of %ld frames", done, frames);
    struct snd_pcm_status st;
    memset(&st, 0, sizeof(st));
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_STATUS, &st) == 0 && st.state == SNDRV_PCM_STATE_RUNNING &&
              st.appl_ptr == (unsigned long)frames && st.hw_ptr <= st.appl_ptr && st.hw_ptr > 0,
          "running after the writes (state %d, appl %lu, hw %lu)", st.state, st.appl_ptr, st.hw_ptr);
    struct snd_pcm_sync_ptr sp;
    memset(&sp, 0, sizeof(sp));
    sp.flags = SNDRV_PCM_SYNC_PTR_APPL | SNDRV_PCM_SYNC_PTR_AVAIL_MIN;
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_SYNC_PTR, &sp) == 0 && sp.c.control.appl_ptr == (unsigned long)frames &&
              sp.c.control.avail_min == PERIOD,
          "SYNC_PTR (appl %lu, avail_min %lu)", sp.c.control.appl_ptr, sp.c.control.avail_min);
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_DRAIN) == 0, "DRAIN");
    double took = now_s() - t0;
    memset(&st, 0, sizeof(st));
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_STATUS, &st) == 0 && st.state == SNDRV_PCM_STATE_SETUP &&
              st.hw_ptr == (unsigned long)frames,
          "set up after the drain (state %d, hw %lu)", st.state, st.hw_ptr);
    /* The device consumes frames in real time: playing cannot be much
     * faster than the tone is long. */
    CHECK(took >= ms / 1000.0 * 0.8, "%u ms of sound played in %.0f ms", ms, took * 1000);
    printf("pcm-tone: played %u Hz for %u ms at %u Hz %s (%ld frames, %s) in %.0f ms\n", hz, ms, rate, f->name,
           frames, use_write ? "write" : "WRITEI_FRAMES", took * 1000);
    free(buf);
}

/* An underrun: start on the first frame, then let the device run dry. */
static void underrun(int fd)
{
    if (setup(fd, &FORMATS[0], 48000, 1) != 0)
        return;
    static int16_t quiet[PERIOD * CHANNELS];
    struct snd_xferi x = {.result = 0, .buf = quiet, .frames = PERIOD};
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_WRITEI_FRAMES, &x) == 0 && x.result == PERIOD, "a period of silence");
    struct snd_pcm_status st;
    memset(&st, 0, sizeof(st));
    double t0 = now_s();
    do {
        usleep(20000);
        ioctl(fd, SNDRV_PCM_IOCTL_STATUS, &st);
    } while (st.state == SNDRV_PCM_STATE_RUNNING && now_s() - t0 < 5);
    CHECK(st.state == SNDRV_PCM_STATE_XRUN, "an underrun once the period is played (state %d)", st.state);
    x.result = 0;
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_WRITEI_FRAMES, &x) < 0 && errno == EPIPE, "writes after the underrun: EPIPE");
    struct pollfd pfd = {.fd = fd, .events = POLLOUT};
    CHECK(poll(&pfd, 1, 0) == 1 && (pfd.revents & POLLERR), "poll reports the underrun (%#x)", pfd.revents);
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_PREPARE) == 0, "PREPARE after the underrun");
    x.result = 0;
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_WRITEI_FRAMES, &x) == 0 && x.result == PERIOD, "writing again");
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_DROP) == 0, "DROP");
    memset(&st, 0, sizeof(st));
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_STATUS, &st) == 0 && st.state == SNDRV_PCM_STATE_SETUP, "set up after DROP");
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_START) < 0 && errno == EBADFD, "START outside PREPARED: EBADFD");
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_HW_FREE) == 0, "HW_FREE");
    printf("pcm-tone: underrun reported and recovered\n");
}

/* Every open is a substream of its own (the audio server mixes them): up
 * to SUBDEVICES at once, then EBUSY. */
static void subdevices(int first)
{
    int fds[SUBDEVICES + 1];
    int n = 0;
    struct snd_pcm_info info;
    memset(&info, 0, sizeof(info));
    CHECK(ioctl(first, SNDRV_PCM_IOCTL_INFO, &info) == 0 && info.subdevice == 0 &&
              info.subdevices_count == SUBDEVICES,
          "the first open is subdevice 0 of %u", info.subdevices_count);
    for (; n < SUBDEVICES; n++) {
        fds[n] = open(PCM_PATH, O_RDWR);
        if (fds[n] < 0)
            break;
    }
    CHECK(n == SUBDEVICES - 1 && errno == EBUSY, "%d more opens, then EBUSY", n);
    if (n > 0) {
        memset(&info, 0, sizeof(info));
        CHECK(ioctl(fds[0], SNDRV_PCM_IOCTL_INFO, &info) == 0 && info.subdevice == 1, "the second open is subdevice 1");
    }
    while (n > 0)
        close(fds[--n]);
}

/* ---- mixer controls ---- */

static int find_control(int ctl, const char *name, struct snd_ctl_elem_info *out)
{
    memset(out, 0, sizeof(*out));
    out->id.iface = SNDRV_CTL_ELEM_IFACE_MIXER;
    snprintf((char *)out->id.name, sizeof(out->id.name), "%s", name);
    return ioctl(ctl, SNDRV_CTL_IOCTL_ELEM_INFO, out);
}

/* Set the master volume to `vol` through the control device, checking the
 * element list, info, read-back, range checks and the change event. */
static void set_volume(long vol)
{
    int ctl = open(CTL_PATH, O_RDWR);
    int watcher = open(CTL_PATH, O_RDWR | O_NONBLOCK);
    CHECK(ctl >= 0 && watcher >= 0, "open " CTL_PATH " twice");
    if (ctl < 0 || watcher < 0)
        return;
    struct snd_ctl_elem_list list;
    struct snd_ctl_elem_id ids[8];
    memset(&list, 0, sizeof(list));
    list.space = 8;
    list.pids = ids;
    CHECK(ioctl(ctl, SNDRV_CTL_IOCTL_ELEM_LIST, &list) == 0 && list.count >= 2 && list.used == list.count,
          "ELEM_LIST (%u of %u)", list.used, list.count);
    for (unsigned i = 0; i < list.used && i < 8; i++)
        printf("pcm-tone: control %u: %s\n", ids[i].numid, (char *)ids[i].name);
    struct snd_ctl_elem_info info;
    CHECK(find_control(ctl, "Master Playback Volume", &info) == 0 && info.type == SNDRV_CTL_ELEM_TYPE_INTEGER &&
              info.count == 2 && info.value.integer.max >= vol,
          "ELEM_INFO of Master Playback Volume (type %d, count %u, max %ld)", info.type, info.count,
          info.value.integer.max);
    int sub = 1;
    CHECK(ioctl(watcher, SNDRV_CTL_IOCTL_SUBSCRIBE_EVENTS, &sub) == 0, "SUBSCRIBE_EVENTS");
    struct snd_ctl_elem_value v;
    memset(&v, 0, sizeof(v));
    v.id.numid = info.id.numid;
    v.value.integer.value[0] = info.value.integer.max + 1;
    v.value.integer.value[1] = vol;
    CHECK(ioctl(ctl, SNDRV_CTL_IOCTL_ELEM_WRITE, &v) < 0 && errno == EINVAL, "a volume out of range: EINVAL");
    v.value.integer.value[0] = vol;
    CHECK(ioctl(ctl, SNDRV_CTL_IOCTL_ELEM_WRITE, &v) == 0, "ELEM_WRITE of the volume");
    memset(&v.value, 0, sizeof(v.value));
    CHECK(ioctl(ctl, SNDRV_CTL_IOCTL_ELEM_READ, &v) == 0 && v.value.integer.value[0] == vol &&
              v.value.integer.value[1] == vol,
          "ELEM_READ gives %ld/%ld", v.value.integer.value[0], v.value.integer.value[1]);
    struct snd_ctl_event ev[4];
    memset(ev, 0, sizeof(ev));
    ssize_t got = read(watcher, ev, sizeof(ev));
    if (vol != 100)
        CHECK(got == sizeof(ev[0]) && ev[0].type == SNDRV_CTL_EVENT_ELEM &&
                  (ev[0].data.elem.mask & SNDRV_CTL_EVENT_MASK_VALUE) && ev[0].data.elem.id.numid == info.id.numid,
              "one value event for the volume (read %zd)", got);
    struct snd_ctl_elem_info sw;
    CHECK(find_control(ctl, "Master Playback Switch", &sw) == 0 && sw.type == SNDRV_CTL_ELEM_TYPE_BOOLEAN,
          "ELEM_INFO of Master Playback Switch");
    printf("pcm-tone: Master Playback Volume set to %ld of %ld\n", vol, info.value.integer.max);
    close(watcher);
    close(ctl);
}

/* ---- mmap ---- */

/* Play `ms` of `hz` at `rate` through the mapped ring buffer. */
static void play_mmap(unsigned rate, unsigned hz, unsigned ms)
{
    const struct format *f = &FORMATS[0];
    int fd = open(PCM_PATH, O_RDWR);
    CHECK(fd >= 0, "open " PCM_PATH " for mmap");
    if (fd < 0)
        return;
    if (setup_access(fd, f, rate, ~0ul >> 1, SNDRV_PCM_ACCESS_MMAP_INTERLEAVED) != 0) {
        close(fd);
        return;
    }
    const long buffer = PERIOD * PERIODS;
    const int fb = f->bytes * CHANNELS;
    volatile struct snd_pcm_mmap_status *status =
        mmap(NULL, 4096, PROT_READ, MAP_SHARED, fd, SNDRV_PCM_MMAP_OFFSET_STATUS);
    volatile struct snd_pcm_mmap_control *control =
        mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, SNDRV_PCM_MMAP_OFFSET_CONTROL);
    unsigned char *data = mmap(NULL, buffer * fb, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    CHECK(status != MAP_FAILED && control != MAP_FAILED && data != MAP_FAILED, "mmap of status, control and data");
    CHECK(mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, fd, 0) == MAP_FAILED, "a private mapping is refused");
    if (status == MAP_FAILED || control == MAP_FAILED || data == MAP_FAILED) {
        close(fd);
        return;
    }
    CHECK(status->state == SNDRV_PCM_STATE_PREPARED && status->hw_ptr == 0 && control->appl_ptr == 0,
          "the status page shows PREPARED (state %d, hw %lu, appl %lu)", status->state, status->hw_ptr,
          control->appl_ptr);
    long frames = (long)rate * ms / 1000;
    long done = 0;
    int started = 0, saw_running = 0;
    double t0 = now_s();
    while (done < frames) {
        long hw = status->hw_ptr;
        long appl = done;
        long avail = buffer - (appl - hw);
        if (avail < PERIOD) {
            if (!started) {
                CHECK(ioctl(fd, SNDRV_PCM_IOCTL_START) == 0, "START with a full mapped buffer");
                started = 1;
                continue;
            }
            struct pollfd pfd = {.fd = fd, .events = POLLOUT};
            if (poll(&pfd, 1, 5000) != 1 || (pfd.revents & POLLERR)) {
                CHECK(0, "poll for room in the mapped buffer: revents %#x, state %d", pfd.revents, status->state);
                break;
            }
            saw_running |= status->state == SNDRV_PCM_STATE_RUNNING && status->hw_ptr > 0;
            continue;
        }
        long n = frames - done < PERIOD ? frames - done : PERIOD;
        for (long i = 0; i < n; i++) {
            double v = amplitude / 32768.0 * sin(2 * M_PI * hz * (double)(done + i) / rate);
            unsigned char *at = data + ((done + i) % buffer) * fb;
            for (int c = 0; c < CHANNELS; c++)
                put_sample(f, at + c * f->bytes, v);
        }
        done += n;
        if (done < frames / 2) {
            control->appl_ptr = done;
        } else {
            struct snd_pcm_sync_ptr sp;
            memset(&sp, 0, sizeof(sp));
            sp.flags = SNDRV_PCM_SYNC_PTR_AVAIL_MIN;
            sp.c.control.appl_ptr = done;
            CHECK(ioctl(fd, SNDRV_PCM_IOCTL_SYNC_PTR, &sp) == 0 && sp.c.control.appl_ptr == (unsigned long)done,
                  "SYNC_PTR moves appl to %ld (got %lu)", done, sp.c.control.appl_ptr);
        }
    }
    if (!started)
        CHECK(ioctl(fd, SNDRV_PCM_IOCTL_START) == 0, "START");
    CHECK(done == frames, "mapped %ld of %ld frames", done, frames);
    CHECK(saw_running, "the status page showed the stream running");
    CHECK(control->appl_ptr == (unsigned long)frames, "the control page follows SYNC_PTR (appl %lu)", control->appl_ptr);
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_DRAIN) == 0, "DRAIN of the mapped stream");
    double took = now_s() - t0;
    CHECK(status->state == SNDRV_PCM_STATE_SETUP && status->hw_ptr == (unsigned long)frames,
          "drained (state %d, hw %lu)", status->state, status->hw_ptr);
    CHECK(took >= ms / 1000.0 * 0.8, "%u ms of mapped sound played in %.0f ms", ms, took * 1000);
    printf("pcm-tone: played %u Hz for %u ms at %u Hz through mmap in %.0f ms\n", hz, ms, rate, took * 1000);
    munmap(data, buffer * fb);
    munmap((void *)control, 4096);
    munmap((void *)status, 4096);
    close(fd);
}

/* ---- two processes at once ---- */

static void mix(unsigned hz1, unsigned hz2, unsigned ms)
{
    double saved = amplitude;
    amplitude = 8192.0;
    fflush(stdout);
    pid_t child = fork();
    if (child == 0) {
        int fd = open(PCM_PATH, O_RDWR);
        CHECK(fd >= 0, "the child opens " PCM_PATH);
        if (fd >= 0) {
            play(fd, &FORMATS[0], 44100, hz2, ms, 1);
            close(fd);
        }
        _exit(failed ? 1 : 0);
    }
    CHECK(child > 0, "fork");
    int fd = open(PCM_PATH, O_RDWR);
    CHECK(fd >= 0, "the parent opens " PCM_PATH);
    if (fd >= 0) {
        play(fd, &FORMATS[0], 48000, hz1, ms, 0);
        close(fd);
    }
    int status = 0;
    CHECK(child > 0 && waitpid(child, &status, 0) == child && WIFEXITED(status) && WEXITSTATUS(status) == 0,
          "the child played its tone (status %#x)", status);
    printf("pcm-tone: %u Hz and %u Hz played at once by two processes\n", hz1, hz2);
    amplitude = saved;
}

/* ---- capture ---- */

static void capture(unsigned rate, unsigned ms)
{
    const struct format *f = &FORMATS[0];
    int fd = open(CAPTURE_PATH, O_RDWR);
    CHECK(fd >= 0, "open " CAPTURE_PATH);
    if (fd < 0)
        return;
    struct snd_pcm_info info;
    memset(&info, 0, sizeof(info));
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_INFO, &info) == 0 && info.stream == SNDRV_PCM_STREAM_CAPTURE, "a capture stream");
    int16_t probe[2];
    CHECK(write(fd, probe, sizeof(probe)) < 0, "writing to a capture stream fails");
    if (setup(fd, f, rate, 1) != 0) {
        close(fd);
        return;
    }
    long frames = (long)rate * ms / 1000;
    int fb = f->bytes * CHANNELS;
    int16_t *buf = malloc(frames * fb);
    long done = 0;
    double t0 = now_s();
    int use_read = 0;
    while (done < frames && buf) {
        long chunk = frames - done < PERIOD ? frames - done : PERIOD;
        long got;
        if (use_read) {
            ssize_t r = read(fd, (unsigned char *)buf + done * fb, chunk * fb);
            got = r < 0 ? -1 : r / fb;
        } else {
            struct snd_xferi x = {.result = 0, .buf = (unsigned char *)buf + done * fb, .frames = chunk};
            got = ioctl(fd, SNDRV_PCM_IOCTL_READI_FRAMES, &x) == 0 ? x.result : -1;
        }
        if (got <= 0) {
            CHECK(0, "%s of %ld frames gave %ld", use_read ? "read" : "READI_FRAMES", chunk, got);
            break;
        }
        done += got;
        use_read = done >= frames / 2;
        if (done == got) {
            struct pollfd pfd = {.fd = fd, .events = POLLIN};
            CHECK(poll(&pfd, 1, 5000) == 1 && (pfd.revents & POLLIN), "poll for captured frames (%#x)", pfd.revents);
        }
    }
    double took = now_s() - t0;
    CHECK(done == frames, "captured %ld of %ld frames", done, frames);
    CHECK(took >= ms / 1000.0 * 0.8, "%u ms captured in %.0f ms", ms, took * 1000);
    struct snd_pcm_status st;
    memset(&st, 0, sizeof(st));
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_STATUS, &st) == 0 && st.state == SNDRV_PCM_STATE_RUNNING &&
              st.appl_ptr == (unsigned long)frames && st.hw_ptr >= st.appl_ptr,
          "capturing (state %d, appl %lu, hw %lu)", st.state, st.appl_ptr, st.hw_ptr);
    int peak = 0;
    for (long i = 0; buf && i < done * CHANNELS; i++)
        peak = abs(buf[i]) > peak ? abs(buf[i]) : peak;
    printf("pcm-tone: captured %ld frames at %u Hz in %.0f ms, peak %d\n", done, rate, took * 1000, peak);
    free(buf);
    /* Nobody reading: the buffer fills up, an overrun. */
    usleep(400000);
    memset(&st, 0, sizeof(st));
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_STATUS, &st) == 0 && st.state == SNDRV_PCM_STATE_XRUN,
          "an overrun once the buffer is full (state %d)", st.state);
    int16_t one[PERIOD * CHANNELS];
    struct snd_xferi x = {.result = 0, .buf = one, .frames = PERIOD};
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_READI_FRAMES, &x) < 0 && errno == EPIPE, "reads after the overrun: EPIPE");
    struct pollfd pfd = {.fd = fd, .events = POLLIN};
    CHECK(poll(&pfd, 1, 0) == 1 && (pfd.revents & POLLERR), "poll reports the overrun (%#x)", pfd.revents);
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_PREPARE) == 0 && ioctl(fd, SNDRV_PCM_IOCTL_START) == 0, "PREPARE and START again");
    x.result = 0;
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_READI_FRAMES, &x) == 0 && x.result == PERIOD, "reading again");
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_DROP) == 0 && ioctl(fd, SNDRV_PCM_IOCTL_HW_FREE) == 0, "DROP and HW_FREE");
    printf("pcm-tone: overrun reported and recovered\n");
    close(fd);
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    char *defaults[] = {"48000:440:1500", "44100:1000:1500:w"};
    char **tones = argc > 1 ? argv + 1 : defaults;
    int count = argc > 1 ? argc - 1 : 2;

    control_checks();
    int fd = open(PCM_PATH, O_RDWR);
    CHECK(fd >= 0, "open " PCM_PATH);
    if (fd < 0) {
        printf("pcm-tone: done, %d failed\n", failed);
        return 1;
    }
    subdevices(fd);
    int version = 0;
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_PVERSION, &version) == 0 && version >= SNDRV_PROTOCOL_VERSION(2, 0, 0),
          "SNDRV_PCM_IOCTL_PVERSION gave %#x", version);
    struct snd_pcm_info info;
    memset(&info, 0, sizeof(info));
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_INFO, &info) == 0 && info.stream == SNDRV_PCM_STREAM_PLAYBACK,
          "SNDRV_PCM_IOCTL_INFO");
    struct snd_xferi x = {.result = 0, .buf = &version, .frames = 1};
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_WRITEI_FRAMES, &x) < 0 && errno == EBADFD, "writing before HW_PARAMS: EBADFD");

    for (int i = 0; i < count; i++) {
        unsigned rate = 0, hz = 0, ms = 0, vol = 0;
        int used = 0;
        if (strncmp(tones[i], "mmap:", 5) == 0) {
            if (sscanf(tones[i] + 5, "%u:%u:%u:%u", &rate, &hz, &ms, &vol) == 4) {
                set_volume(vol);
                play_mmap(rate, hz, ms);
                set_volume(100);
            } else {
                CHECK(0, "bad mmap tone %s", tones[i]);
            }
            continue;
        }
        if (strncmp(tones[i], "mix:", 4) == 0) {
            if (sscanf(tones[i] + 4, "%u:%u:%u", &rate, &hz, &ms) == 3)
                mix(rate, hz, ms);
            else
                CHECK(0, "bad mix %s", tones[i]);
            continue;
        }
        if (strncmp(tones[i], "capture:", 8) == 0) {
            if (sscanf(tones[i] + 8, "%u:%u", &rate, &ms) == 2)
                capture(rate, ms);
            else
                CHECK(0, "bad capture %s", tones[i]);
            continue;
        }
        if (sscanf(tones[i], "%u:%u:%u%n", &rate, &hz, &ms, &used) < 3) {
            CHECK(0, "bad tone %s", tones[i]);
            continue;
        }
        /* The rest: "", ":w", ":w:FORMAT" or "::FORMAT". */
        const char *rest = tones[i] + used;
        int use_write = strncmp(rest, ":w", 2) == 0;
        const char *name = "s16";
        const char *colon = rest[0] == ':' ? strchr(rest + 1, ':') : NULL;
        if (colon)
            name = colon + 1;
        const struct format *f = NULL;
        for (size_t k = 0; k < sizeof(FORMATS) / sizeof(FORMATS[0]); k++)
            if (strcmp(FORMATS[k].name, name) == 0)
                f = &FORMATS[k];
        if (!f) {
            CHECK(0, "unknown format %s", name);
            continue;
        }
        play(fd, f, rate, hz, ms, use_write);
    }
    underrun(fd);
    close(fd);
    /* Closed: the device can be opened again. */
    fd = open(PCM_PATH, O_RDWR | O_NONBLOCK);
    CHECK(fd >= 0, "open again after close");
    close(fd);
    printf("pcm-tone: done, %d failed\n", failed);
    return failed ? 1 : 0;
}
