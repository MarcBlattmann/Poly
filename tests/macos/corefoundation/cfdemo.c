/*
 * A small C program that uses Poly's own CoreFoundation the ordinary way
 * (ADR 0260): CFString, CFArray, CFStringCreateWithFormat and a repeating
 * CFRunLoop timer that stops its run loop after three firings. It prints
 * with printf, so it runs through Poly's dyld, libSystem and
 * CoreFoundation together. Written for Poly; no code is copied from
 * anywhere.
 */
#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>

static const char *text(CFStringRef s, char *buf, CFIndex n) {
    if (!s || !CFStringGetCString(s, buf, n, kCFStringEncodingUTF8)) return "<none>";
    return buf;
}

static int fired;

static void tick(CFRunLoopTimerRef timer, void *info) {
    (void)info;
    if (++fired == 3) {
        CFRunLoopTimerInvalidate(timer);
        CFRunLoopStop(CFRunLoopGetCurrent());
    }
}

int main(void) {
    char a[256], b[256];
    CFStringRef hello = CFStringCreateWithCString(kCFAllocatorDefault, "hello from CoreFoundation on Poly",
                                                  kCFStringEncodingUTF8);
    printf("cfdemo: %s\n", text(hello, a, sizeof a));

    CFMutableArrayRef words = CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    CFArrayAppendValue(words, CFSTR("alpha"));
    CFArrayAppendValue(words, CFSTR("beta"));
    CFArrayAppendValue(words, CFSTR("gamma"));
    CFStringRef listed = CFStringCreateByCombiningStrings(kCFAllocatorDefault, words, CFSTR(", "));
    printf("cfdemo: array of %ld: %s\n", (long)CFArrayGetCount(words), text(listed, a, sizeof a));
    CFStringRef joined = CFStringCreateByCombiningStrings(kCFAllocatorDefault, words, CFSTR("-"));
    printf("cfdemo: joined: %s\n", text(joined, a, sizeof a));

    CFStringRef formatted =
        CFStringCreateWithFormat(kCFAllocatorDefault, NULL, CFSTR("%d %.2f [%#x] <%@>"), 42, 3.14159, 42, CFSTR("one"));
    printf("cfdemo: format: %s\n", text(formatted, b, sizeof b));

    CFRunLoopTimerRef timer = CFRunLoopTimerCreate(kCFAllocatorDefault, CFAbsoluteTimeGetCurrent() + 0.05, 0.05, 0,
                                                   0, tick, NULL);
    CFRunLoopAddTimer(CFRunLoopGetCurrent(), timer, kCFRunLoopDefaultMode);
    SInt32 r = CFRunLoopRunInMode(kCFRunLoopDefaultMode, 5.0, false);
    printf("cfdemo: timer fired %d times\n", fired);
    printf("cfdemo: run loop result %d (%s)\n", (int)r, r == kCFRunLoopRunStopped ? "stopped" : "not stopped");

    CFRelease(timer);
    CFRelease(formatted);
    CFRelease(joined);
    CFRelease(listed);
    CFRelease(words);
    CFRelease(hello);
    printf("cfdemo: done\n");
    return fired == 3 && r == kCFRunLoopRunStopped ? 0 : 1;
}
