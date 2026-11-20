/* poly-mac: coretext */
/* The triangle V is move (0,0), line (35,70), line (70,0), close; the composite W has two
 * subpaths, the second the first shifted by 70; the composite e-acute has the subpath of a
 * (5,0..45,50) and of the mark shifted by (15,52); the composite's path equals the
 * paths of its parts moved. */
#include "poly_mct.h"

typedef struct { int n; int kind[40]; double x[40], y[40]; } Path;
static void cb(void *info, const CGPathElement *e) {
    Path *p = info;
    int pts = e->type == kCGPathElementMoveToPoint || e->type == kCGPathElementAddLineToPoint ? 1 : 0;
    p->kind[p->n] = e->type;
    p->x[p->n] = pts ? e->points[0].x : 0;
    p->y[p->n] = pts ? e->points[0].y : 0;
    p->n++;
}
static void get(Path *p, CGPathRef path) { p->n = 0; CGPathApply(path, p, cb); }

int test_main(void) {
    CTFontRef f = pt_font(0, 100);
    Path p;
    get(&p, CTFontCreatePathForGlyph(f, 4, NULL));
    CK(p.n == 4 && p.kind[0] == kCGPathElementMoveToPoint && p.kind[3] == kCGPathElementCloseSubpath);
    CKF(p.x[0], 0); CKF(p.y[0], 0); CKF(p.x[1], 35); CKF(p.y[1], 70); CKF(p.x[2], 70); CKF(p.y[2], 0);
    get(&p, CTFontCreatePathForGlyph(f, 5, NULL));
    CK(p.n == 8 && p.kind[4] == kCGPathElementMoveToPoint);
    CKF(p.x[4], 70); CKF(p.y[4], 0); CKF(p.x[5], 105); CKF(p.y[5], 70); CKF(p.x[6], 140); CKF(p.y[6], 0);
    get(&p, CTFontCreatePathForGlyph(f, 8, NULL));
    CK(p.n == 10 && p.kind[5] == kCGPathElementMoveToPoint);
    CKF(p.x[0], 5); CKF(p.y[0], 0); CKF(p.x[2], 45); CKF(p.y[2], 50);
    CKF(p.x[5], 15); CKF(p.y[5], 52); CKF(p.x[7], 25); CKF(p.y[7], 62);
    CGRect b = CGPathGetBoundingBox(CTFontCreatePathForGlyph(f, 8, NULL));
    CKF(b.size.height, 62);
    return finish();
}
