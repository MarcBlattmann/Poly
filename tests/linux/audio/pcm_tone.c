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

#define CTL_PATH "/dev/snd/controlC0"
#define PCM_PATH "/dev/snd/pcmC0D0p"
#define CHANNELS 2
#define PERIOD 1024
#define PERIODS 4
#define AMPLITUDE 16384.0

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
static int setup(int fd, const struct format *f, unsigned rate, unsigned long start_threshold)
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

    params_any(&p);
    mask_only(mask(&p, SNDRV_PCM_HW_PARAM_ACCESS), SNDRV_PCM_ACCESS_RW_INTERLEAVED);
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
    CHECK(ioctl(fd, SNDRV_PCM_IOCTL_STATUS, &st) == 0 && st.state == SNDRV_PCM_STATE_PREPARED &&
              st.avail == PERIOD * PERIODS,
          "prepared with an empty buffer (state %d, avail %lu)", st.state, st.avail);
    return 0;
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
        double v = AMPLITUDE / 32768.0 * sin(2 * M_PI * hz * (double)i / rate);
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
    CHECK(open(PCM_PATH, O_RDWR) < 0 && errno == EBUSY, "a second open: EBUSY");
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
        unsigned rate = 0, hz = 0, ms = 0;
        int used = 0;
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
