/* dispatch_queue_create: the label is copied (any length), NULL
 * and empty labels give an empty label, every call makes a new queue. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_queue_t s = dispatch_queue_create("com.poly.serial", NULL);
    check("a serial queue is made", s != NULL, 0);
    check_str("its label", dispatch_queue_get_label(s), "com.poly.serial");
    dispatch_queue_t s2 = dispatch_queue_create("com.poly.serial", DISPATCH_QUEUE_SERIAL);
    check("the same label gives a different queue", s2 != NULL && s2 != s, 0);
    check("DISPATCH_QUEUE_SERIAL is NULL", DISPATCH_QUEUE_SERIAL == NULL, 0);
    check("DISPATCH_QUEUE_CONCURRENT is not NULL", DISPATCH_QUEUE_CONCURRENT != NULL, 0);

    dispatch_queue_t c = dispatch_queue_create("com.poly.conc", DISPATCH_QUEUE_CONCURRENT);
    check("a concurrent queue is made", c != NULL, 0);
    check_str("its label", dispatch_queue_get_label(c), "com.poly.conc");

    dispatch_queue_t n = dispatch_queue_create(NULL, NULL);
    check("a NULL label still makes a queue", n != NULL, 0);
    check("its label is not NULL but empty", dispatch_queue_get_label(n) != NULL && dispatch_queue_get_label(n)[0] == 0, 0);
    dispatch_queue_t e = dispatch_queue_create("", NULL);
    check_str("an empty label", dispatch_queue_get_label(e), "");

    char buf[8] = "abc";
    dispatch_queue_t cp = dispatch_queue_create(buf, NULL);
    strcpy(buf, "xyz");
    check_str("the label is copied: changing the caller's string changes nothing", dispatch_queue_get_label(cp), "abc");

    char big[300];
    memset(big, 'a', sizeof big);
    big[299] = 0;
    dispatch_queue_t b = dispatch_queue_create(big, NULL);
    check("a 299 byte label is kept whole", strlen(dispatch_queue_get_label(b)) == 299, (long long)strlen(dispatch_queue_get_label(b)));
    check("... and is a copy, not the caller's buffer", dispatch_queue_get_label(b) != big, 0);

    check_str("DISPATCH_CURRENT_QUEUE_LABEL on the main thread names the main queue", dispatch_queue_get_label(DISPATCH_CURRENT_QUEUE_LABEL), "com.apple.main-thread");
    return finish();
}
