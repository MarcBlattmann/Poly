/* poly-mac: appkit */
/* The delegate methods of the launch, in the order they arrive inside run: applicationWillFinishLaunching:,
 * then applicationDidFinishLaunching:, each once; only the methods the delegate implements are
 * called (no error for the others, and a delegate with none of them works too); the
 * notification object of both is the application; the notification names are
 * NSApplicationWillFinishLaunchingNotification and NSApplicationDidFinishLaunchingNotification. */
#include "poly_mak.h"

static int will_obj_is_app, did_obj_is_app;
static char log_[100];
static int n;
static void add(char c) { log_[n++] = c; log_[n] = 0; }

@interface Both : NSObject <NSApplicationDelegate>
@end
@implementation Both
- (void)applicationWillFinishLaunching:(NSNotification *)x { will_obj_is_app = x.object == NSApp; add('w'); CK(ns_is(x.name, "NSApplicationWillFinishLaunchingNotification")); }
- (void)applicationDidFinishLaunching:(NSNotification *)x { did_obj_is_app = x.object == NSApp; add('d'); CK(ns_is(x.name, "NSApplicationDidFinishLaunchingNotification")); [NSApp stop:nil]; [NSApp postEvent:ak_event(0, 0) atStart:YES]; }
@end

@interface Bare : NSObject <NSApplicationDelegate>
@end
@implementation Bare
@end

int test_main(void) {
    NSApplication *a = ak_init();
    Both *d = [[Both alloc] init];
    a.delegate = d;
    CK(a.delegate == d);
    [a run];
    CK(str_eq(log_, "wd"));
    CK(will_obj_is_app && did_obj_is_app);
    Bare *b = [[Bare alloc] init];
    a.delegate = b;
    CK(a.delegate == b);
    ak_stop_run_after(0.2);
    [a run];
    CK(str_eq(log_, "wd"));
    return finish();
}
