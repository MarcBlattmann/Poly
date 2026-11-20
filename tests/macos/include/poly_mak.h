/*
 * Support for macOS test programs about AppKit (ADR 0182, the AppKit basics batch). A
 * test that includes this header says so in its header comment with the line
 * `poly-mac: appkit` (`mac build` then compiles the `.c` file as Objective-C with ARC,
 * links it with -framework AppKit and Foundation, and lets it import AppKit's,
 * Foundation's, CoreFoundation's, CoreGraphics', CoreText's and QuartzCore's own symbols
 * and the Objective-C runtime's; all other tests import nothing, see poly_mtest.h), and
 * defines `int test_main(void)`. Classes, constants and types come from the public SDK
 * headers (<AppKit/AppKit.h>). It includes poly_mfn.h (`ns_is`, `CK`, `check`,
 * `value`, `finish`) and adds `CKF`, `near` and `value_f` (numbers to 1/100; the test runs inside an autorelease pool).
 *
 * The tests run as a program that is not in an application bundle: its activation policy is
 * NSApplicationActivationPolicyProhibited (no Dock icon, never active, no window becomes
 * key), and the Mac's console session is the one every AppKit call needs (the window
 * server). The windows the tests make are kept invisible: `ak_window` gives them an alpha
 * value of 0 and makes them ignore the mouse, and a window that is ordered front is
 * ordered out or closed before the test ends (ordering front moves a window to the
 * visible part of the screen, invisibly at alpha 0). Nothing here takes the keyboard or
 * the mouse from the person at the Mac.
 *
 * What is reported does not go through the code under test: output is the raw `write`
 * system call of poly_mtest.h (see poly_mcf.h). Never print addresses, window numbers,
 * times or anything else that differs between runs; every test runs several times per
 * architecture and must print the same bytes. A test starts in a fresh, empty scratch
 * directory with an empty environment. A test about the pixels of a view draws it into a
 * bitmap it makes itself (`ak_render`), tagged with the sRGB colour space, so that colours
 * given in sRGB come out as the same bytes whatever the display's profile; the appearance
 * (light or dark) is always set by the test, never taken from the Mac's setting. The
 * bitmaps hold premultiplied RGBA bytes, row 0 at the top.
 *
 * `ak_init` also starts a watchdog thread that ends the process with status 3 after 12 seconds,
 * so that a test of `-[NSApplication run]` that does not return fails instead of hanging.
 *
 * Written for Poly from public sources and from observation only. No Apple or third-party
 * source (including GNUstep and other UI toolkits) was read; no code is copied from anywhere.
 */
#ifndef POLY_MAK_H
#define POLY_MAK_H

#import <AppKit/AppKit.h>
#include "poly_mfn.h"

/* Whether `v` is within 0.01 of `want`; `CKF(value, want)` checks a number with it and shows
 * the value times 100 when it fails; `value_f` prints a number rounded to 1/100 with two
 * places (never -0.00), so float noise does not matter. */
static __attribute__((unused)) int near(double v, double want) { return v > want - 0.01 && v < want + 0.01; }
#define CKF(v, want) check(#v " is " #want, near((double)(v), (double)(want)), (i64)((v) * 100))
static __attribute__((unused)) void value_f(const char *name, double v) {
    long long m = (long long)(v * 100.0 + (v < 0 ? -0.5 : 0.5));
    char buf[40];
    int n = 0, neg = m < 0;
    unsigned long long u = neg ? (unsigned long long)(-m) : (unsigned long long)m;
    char t[24];
    int k = 0;
    do t[k++] = (char)('0' + u % 10), u /= 10;
    while (u);
    while (k < 3) t[k++] = '0';
    if (neg && m != 0) buf[n++] = '-';
    for (int i = k - 1; i >= 2; i--) buf[n++] = t[i];
    buf[n++] = '.';
    for (int i = 1; i >= 0; i--) buf[n++] = t[i];
    buf[n] = 0;
    value_str(name, buf);
}

static __attribute__((unused)) NSApplication *ak_init(void) {
    static int started;
    if (!started) {
        started = 1;
        [NSThread detachNewThreadWithBlock:^{
            [NSThread sleepForTimeInterval:12];
            put("not ok watchdog: the test did not finish in 12 seconds\n");
            poly_exit(3);
        }];
    }
    return [NSApplication sharedApplication];
}

/* An invisible window (alpha 0, mouse clicks pass through, not released when closed) with
 * the style mask and content rectangle given. */
static __attribute__((unused)) NSWindow *ak_window(NSWindowStyleMask mask, NSRect content) {
    ak_init();
    NSWindow *w = [[NSWindow alloc] initWithContentRect:content styleMask:mask backing:NSBackingStoreBuffered defer:NO];
    w.releasedWhenClosed = NO;
    w.alphaValue = 0;
    w.ignoresMouseEvents = YES;
    return w;
}

/* Run the main run loop for `seconds`. */
static __attribute__((unused)) void ak_spin(double seconds) {
    [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:seconds]];
}

/* A dummy application-defined event (subtype `subtype`, data1 `d1`), as posted to wake the
 * event loop. */
static __attribute__((unused)) NSEvent *ak_event(short subtype, NSInteger d1) {
    return [NSEvent otherEventWithType:NSEventTypeApplicationDefined location:NSZeroPoint modifierFlags:0 timestamp:0 windowNumber:0 context:nil subtype:subtype data1:d1 data2:0];
}

/* Make `[NSApp run]` return `seconds` from now: a timer on the main run loop sends stop: and
 * posts a dummy event (stop: alone only takes effect when the next event arrives). */
static __attribute__((unused)) void ak_stop_run_after(double seconds) {
    NSTimer *t = [NSTimer timerWithTimeInterval:seconds repeats:NO block:^(NSTimer *x) {
        (void)x;
        [NSApp stop:nil];
        [NSApp postEvent:ak_event(0, 0) atStart:YES];
    }];
    [[NSRunLoop currentRunLoop] addTimer:t forMode:NSRunLoopCommonModes];
}

static __attribute__((unused)) NSAppearance *ak_light(void) { return [NSAppearance appearanceNamed:NSAppearanceNameAqua]; }
static __attribute__((unused)) NSAppearance *ak_dark(void) { return [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua]; }

/* A bitmap of w x h pixels, 8 bits per sample, RGBA (premultiplied), tagged sRGB. */
static __attribute__((unused)) NSBitmapImageRep *ak_srgb_rep(NSInteger w, NSInteger h) {
    NSBitmapImageRep *r = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:NULL pixelsWide:w pixelsHigh:h bitsPerSample:8 samplesPerPixel:4 hasAlpha:YES isPlanar:NO colorSpaceName:NSCalibratedRGBColorSpace bytesPerRow:0 bitsPerPixel:0];
    return [r bitmapImageRepByRetaggingWithColorSpace:[NSColorSpace sRGBColorSpace]];
}

/* Draw `view` (as the content view of a borderless invisible window of its size, so it
 * has a window and an appearance) into a new sRGB bitmap at `scale` pixels per point under
 * the appearance `ap`, over the opaque (or any) colour `bg` filled first (nil: over
 * transparent pixels). */
static __attribute__((unused)) NSBitmapImageRep *ak_render_on(NSView *view, NSAppearance *ap, int scale, NSColor *bg) {
    NSWindow *w = ak_window(NSWindowStyleMaskBorderless, NSMakeRect(0, 0, view.frame.size.width, view.frame.size.height));
    w.appearance = ap;
    w.contentView = view;
    NSBitmapImageRep *r = ak_srgb_rep((NSInteger)(view.bounds.size.width * scale), (NSInteger)(view.bounds.size.height * scale));
    r.size = view.bounds.size;
    NSGraphicsContext *gc = [NSGraphicsContext graphicsContextWithBitmapImageRep:r];
    [NSGraphicsContext saveGraphicsState];
    [NSGraphicsContext setCurrentContext:gc];
    if (bg) {
        [bg setFill];
        NSRectFillUsingOperation(NSMakeRect(0, 0, view.bounds.size.width, view.bounds.size.height), NSCompositingOperationCopy);
    }
    [ap performAsCurrentDrawingAppearance:^{
        [view displayRectIgnoringOpacity:view.bounds inContext:gc];
    }];
    [NSGraphicsContext restoreGraphicsState];
    return r;
}

/* The same over transparent pixels. */
static __attribute__((unused)) NSBitmapImageRep *ak_render(NSView *view, NSAppearance *ap, int scale) { return ak_render_on(view, ap, scale, nil); }

/* The 64-bit FNV-1a hash of the bitmap's bytes (rows of bytesPerRow, top row first). */
static __attribute__((unused)) unsigned long long ak_hash(NSBitmapImageRep *r) {
    unsigned long long h = 1469598103934665603ULL;
    const unsigned char *p = r.bitmapData;
    for (size_t i = 0, n = (size_t)r.bytesPerRow * (size_t)r.pixelsHigh; i < n; i++) h = (h ^ p[i]) * 1099511628211ULL;
    return h;
}

/* The pixel at (x, y), row 0 the top, as 0xRRGGBBAA. */
static __attribute__((unused)) unsigned ak_px(NSBitmapImageRep *r, int x, int y) {
    const unsigned char *p = r.bitmapData + (size_t)y * (size_t)r.bytesPerRow + (size_t)x * 4;
    return ((unsigned)p[0] << 24) | ((unsigned)p[1] << 16) | ((unsigned)p[2] << 8) | p[3];
}

/* Print `name` and the pixels of rows y0..y1-1, columns x0..x1-1 as 8 hex digits per
 * pixel. */
static __attribute__((unused)) void ak_dump(const char *name, NSBitmapImageRep *r, int x0, int y0, int x1, int y1) {
    static const char H[] = "0123456789abcdef";
    for (int y = y0; y < y1; y++) {
        char row[400], label[64];
        int j = 0;
        for (int x = x0; x < x1 && j < 380; x++) {
            unsigned v = ak_px(r, x, y);
            for (int s = 28; s >= 0; s -= 4) row[j++] = H[(v >> s) & 15];
            row[j++] = ' ';
        }
        row[j] = 0;
        int k = 0;
        for (const char *p = name; *p && k < 40; p++) label[k++] = *p;
        label[k++] = '.';
        label[k++] = (char)('0' + y / 10);
        label[k++] = (char)('0' + y % 10);
        label[k] = 0;
        value_str(label, row);
    }
}

/* The sRGB components of a colour (resolved for the appearance `ap`) as 0..255 integers
 * with the alpha, rounded to the nearest. */
static __attribute__((unused)) void ak_rgba(NSColor *c, NSAppearance *ap, int out[4]) {
    __block int r = 0, g = 0, b = 0, a = 0;
    [ap performAsCurrentDrawingAppearance:^{
        NSColor *s = [c colorUsingColorSpace:[NSColorSpace sRGBColorSpace]];
        r = (int)(s.redComponent * 255.0 + 0.5);
        g = (int)(s.greenComponent * 255.0 + 0.5);
        b = (int)(s.blueComponent * 255.0 + 0.5);
        a = (int)(s.alphaComponent * 255.0 + 0.5);
    }];
    out[0] = r;
    out[1] = g;
    out[2] = b;
    out[3] = a;
}

#endif
