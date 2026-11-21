/* poly-mac: appkit */
/* The system colours in the LIGHT appearance (NSAppearanceNameAqua; SYSTEM-DEPENDENT, sRGB 0..255 with alpha,
 * printed as "r g b a"): labelColor 0 0 0 216, controlTextColor 0 0 0 216, secondaryLabelColor 0 0 0 127,
 * disabledControlTextColor 0 0 0 63, placeholderTextColor 0 0 0 127, separatorColor 0 0 0 25, windowBackgroundColor
 * 255 255 255 255, textBackgroundColor 255 255 255 255, controlBackgroundColor 255 255 255 255, controlAccentColor
 * 0 122 255 255, systemRedColor 255 56 60 255, systemBlueColor 0 136 255 255, linkColor 0 104 218 255,
 * selectedTextBackgroundColor 179 215 255 255, gridColor 230 230 230 255, shadowColor 0 0 0 255. What is fixed
 * is that the colour of the text is black with partial alpha and the backgrounds are opaque. */
#include "poly_mak.h"

static void rgba(const char *name, NSColor *c) {
    int v[4];
    ak_rgba(c, ak_light(), v);
    char b[48];
    int k = 0;
    for (int i = 0; i < 4; i++) {
        if (v[i] >= 100) b[k++] = (char)('0' + v[i] / 100);
        if (v[i] >= 10) b[k++] = (char)('0' + v[i] / 10 % 10);
        b[k++] = (char)('0' + v[i] % 10);
        b[k++] = ' ';
    }
    b[k] = 0;
    value_str(name, b);
}

int test_main(void) {
    int v[4];
    ak_rgba(NSColor.labelColor, ak_light(), v);
    CK(v[0] == v[1] && v[1] == v[2] && v[0] < 20 && v[3] > 100 && v[3] < 255);
    ak_rgba(NSColor.windowBackgroundColor, ak_light(), v);
    CK(v[3] == 255 && v[0] > 200);
    ak_rgba(NSColor.textBackgroundColor, ak_light(), v);
    CK(v[3] == 255 && v[0] > 200);
    rgba("sys_light_label", NSColor.labelColor);
    rgba("sys_light_controlText", NSColor.controlTextColor);
    rgba("sys_light_secondaryLabel", NSColor.secondaryLabelColor);
    rgba("sys_light_disabledControlText", NSColor.disabledControlTextColor);
    rgba("sys_light_placeholder", NSColor.placeholderTextColor);
    rgba("sys_light_separator", NSColor.separatorColor);
    rgba("sys_light_windowBackground", NSColor.windowBackgroundColor);
    rgba("sys_light_textBackground", NSColor.textBackgroundColor);
    rgba("sys_light_controlBackground", NSColor.controlBackgroundColor);
    rgba("sys_light_accent", NSColor.controlAccentColor);
    rgba("sys_light_systemRed", NSColor.systemRedColor);
    rgba("sys_light_systemBlue", NSColor.systemBlueColor);
    rgba("sys_light_link", NSColor.linkColor);
    rgba("sys_light_selectedTextBackground", NSColor.selectedTextBackgroundColor);
    rgba("sys_light_grid", NSColor.gridColor);
    rgba("sys_light_shadow", NSColor.shadowColor);
    return finish();
}
