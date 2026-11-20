/* CGAffineTransformConcat(t1, t2) = t1 then t2 (row vector convention):
 * exact doubles for a set of pairs, and the identities. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void show(const char *name, CGAffineTransform t) {
    char l[100];
    size_t k = 0;
    for (const char *p = name; *p && k < 60; p++) l[k++] = *p;
    l[k++] = ' ';
    l[k + 1] = 0;
    const char names[6] = {'a', 'b', 'c', 'd', 'x', 'y'};
    double v[6] = {t.a, t.b, t.c, t.d, t.tx, t.ty};
    for (int i = 0; i < 6; i++) {
        l[k] = names[i];
        value_dbl(l, v[i]);
    }
}

int test_main(void) {
    CGAffineTransform tr = CGAffineTransformMakeTranslation(10, 20);
    CGAffineTransform sc = CGAffineTransformMakeScale(2, 3);
    CGAffineTransform ro = CGAffineTransformMakeRotation((CGFloat)0.5);
    show("translate(10,20) then scale(2,3)", CGAffineTransformConcat(tr, sc));
    show("scale(2,3) then translate(10,20)", CGAffineTransformConcat(sc, tr));
    show("rotate(0.5) then translate", CGAffineTransformConcat(ro, tr));
    show("translate then rotate(0.5)", CGAffineTransformConcat(tr, ro));
    show("rotate(0.5) then scale", CGAffineTransformConcat(ro, sc));
    show("the identity then scale", CGAffineTransformConcat(CGAffineTransformIdentity, sc));
    check("concat with the identity is the transform", CGAffineTransformEqualToTransform(CGAffineTransformConcat(sc, CGAffineTransformIdentity), sc), 0);
    CGPoint p = CGPointApplyAffineTransform(CGPointMake(1, 1), CGAffineTransformConcat(tr, sc));
    value_dbl("(1,1) through translate then scale: x", p.x);
    value_dbl("y", p.y);
    check("the identity is IsIdentity", CGAffineTransformIsIdentity(CGAffineTransformIdentity), 0);
    check("the scale is not", !CGAffineTransformIsIdentity(sc), 0);
    return finish();
}
