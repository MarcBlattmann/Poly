/* poly-mac: appkit */
/* The system colours are catalog colours: labelColor has type NSColorTypeCatalog (2), its catalogNameComponent
 * is "System" and colorNameComponent "labelColor"; the same for controlTextColor, windowBackgroundColor,
 * textBackgroundColor, controlAccentColor, separatorColor ("separatorColor"); two calls return equal colours; the
 * component accessors of a catalog colour raise unless it is converted first (colorUsingColorSpace: gives a
 * component colour); the colours are not equal to a fixed colour of the same look. */
#include "poly_mak.h"

int test_main(void) {
    CK(NSColor.labelColor.type == NSColorTypeCatalog && NSColorTypeCatalog == 2);
    CK([NSColor.labelColor.catalogNameComponent isEqualToString:@"System"]);
    CK([NSColor.labelColor.colorNameComponent isEqualToString:@"labelColor"]);
    CK([NSColor.controlTextColor.colorNameComponent isEqualToString:@"controlTextColor"]);
    CK([NSColor.windowBackgroundColor.colorNameComponent isEqualToString:@"windowBackgroundColor"]);
    CK([NSColor.textBackgroundColor.colorNameComponent isEqualToString:@"textBackgroundColor"]);
    CK([NSColor.controlAccentColor.colorNameComponent isEqualToString:@"controlAccentColor"]);
    CK([NSColor.separatorColor.colorNameComponent isEqualToString:@"separatorColor"]);
    CK([NSColor.labelColor isEqual:NSColor.labelColor]);
    CK(![NSColor.labelColor isEqual:NSColor.blackColor]);
    NSString *exc = @"none";
    @try {
        (void)NSColor.labelColor.redComponent;
    } @catch (NSException *e) {
        exc = e.name;
    }
    value_ns("redComponent of a catalog colour raises", exc);
    NSColor *c = [NSColor.labelColor colorUsingColorSpace:[NSColorSpace sRGBColorSpace]];
    CK(c != nil && c.type == NSColorTypeComponentBased);
    return finish();
}
