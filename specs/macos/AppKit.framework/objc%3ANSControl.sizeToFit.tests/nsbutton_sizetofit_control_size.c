/* poly-mac: appkit */
/* The size of a button's fit depends on controlSize: for the title "Hello" the heights are regular 24, small 20,
 * mini 16, large 28 (SYSTEM-DEPENDENT, printed) and the widths 55, 47, 39, 59; larger control sizes give larger
 * buttons (mini < small < regular < large); NSControlSizeRegular 0, Small 1, Mini 2, Large 3; unbordered buttons are
 * 16 high (the text height) and narrower (35 wide for "Hello"); controlSize reads back as set. */
#include "poly_mak.h"

static NSSize fit(NSControlSize s, BOOL bordered) {
    NSButton *b = [[NSButton alloc] initWithFrame:NSMakeRect(0, 0, 10, 10)];
    b.title = @"Hello";
    b.controlSize = s;
    b.bordered = bordered;
    return b.fittingSize;
}

int test_main(void) {
    CK(NSControlSizeRegular == 0 && NSControlSizeSmall == 1 && NSControlSizeMini == 2 && NSControlSizeLarge == 3);
    NSSize r = fit(NSControlSizeRegular, YES), s = fit(NSControlSizeSmall, YES), m = fit(NSControlSizeMini, YES), l = fit(NSControlSizeLarge, YES);
    CK(m.height < s.height && s.height < r.height && r.height < l.height);
    CK(m.width < s.width && s.width < r.width && r.width < l.width);
    NSSize u = fit(NSControlSizeRegular, NO);
    CK(u.width < r.width && u.height < r.height);
    NSButton *b = [[NSButton alloc] initWithFrame:NSMakeRect(0, 0, 10, 10)];
    b.controlSize = NSControlSizeSmall;
    CK(b.controlSize == NSControlSizeSmall);
    value_f("sys_regular_w", r.width);
    value_f("sys_regular_h", r.height);
    value_f("sys_small_w", s.width);
    value_f("sys_small_h", s.height);
    value_f("sys_mini_w", m.width);
    value_f("sys_mini_h", m.height);
    value_f("sys_large_w", l.width);
    value_f("sys_large_h", l.height);
    value_f("sys_unbordered_w", u.width);
    value_f("sys_unbordered_h", u.height);
    return finish();
}
