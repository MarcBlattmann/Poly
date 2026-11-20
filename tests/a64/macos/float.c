#include "rt.h"
static double sqrt_(double x) { return __builtin_sqrt(x); }
static int mandel(double cr, double ci) { double zr = 0, zi = 0; int n = 0; while (n < 200 && zr*zr + zi*zi < 4) { double t = zr*zr - zi*zi + cr; zi = 2*zr*zi + ci; zr = t; n++; } return n; }
typedef struct { double x, y, z, vx, vy, vz, m; } Body;
static void nbody(Body *b, int n, int steps, double dt) {
    for (int s = 0; s < steps; s++) {
        for (int i = 0; i < n; i++) for (int j = i + 1; j < n; j++) {
            double dx = b[i].x - b[j].x, dy = b[i].y - b[j].y, dz = b[i].z - b[j].z;
            double d2 = dx*dx + dy*dy + dz*dz + 0.01, mag = dt / (d2 * sqrt_(d2));
            b[i].vx -= dx * b[j].m * mag; b[i].vy -= dy * b[j].m * mag; b[i].vz -= dz * b[j].m * mag;
            b[j].vx += dx * b[i].m * mag; b[j].vy += dy * b[i].m * mag; b[j].vz += dz * b[i].m * mag;
        }
        for (int i = 0; i < n; i++) { b[i].x += dt * b[i].vx; b[i].y += dt * b[i].vy; b[i].z += dt * b[i].vz; }
    }
}
static float mf[64][64], mg[64][64], mh[64][64];
int run(void) {
    int total = 0;
    for (int y = 0; y < 40; y++) for (int x = 0; x < 80; x++) total += mandel(-2.0 + x * 0.0375, -1.2 + y * 0.06);
    puts_("mandelbrot iterations="); puti(total); nl();
    Body b[5] = {{0,0,0,0,0,0,40},{1,0,0,0,6,0,0.04},{0,2,0,-4.4,0,0,0.01},{-3,0,0.5,0,-3.6,0,0.02},{0,-4,0,3.1,0,0.2,0.005}};
    nbody(b, 5, 2000, 0.001);
    for (int i = 0; i < 5; i++) { putf(b[i].x, 6); putc_(' '); putf(b[i].y, 6); putc_(' '); putf(b[i].z, 6); nl(); }
    for (int i = 0; i < 64; i++) for (int j = 0; j < 64; j++) { mf[i][j] = (float)((i * 7 + j * 3) % 17) * 0.25f; mg[i][j] = (float)((i + 2 * j) % 13) - 6.0f; }
    for (int i = 0; i < 64; i++) for (int j = 0; j < 64; j++) { float s = 0; for (int k = 0; k < 64; k++) s += mf[i][k] * mg[k][j]; mh[i][j] = s; }
    double tr = 0; for (int i = 0; i < 64; i++) tr += mh[i][i] + mh[i][63 - i];
    puts_("matmul trace="); putf(tr, 3); nl();
    double acc = 0; for (int i = 1; i < 1000; i++) { double x = i * 0.01; acc += sqrt_(x) / (1.0 + x * x) - (double)(i64)(x * 3.7) * 0.001; }
    puts_("series="); putf(acc, 9); nl();
    float fsum = 0; for (int i = 1; i < 100000; i++) fsum += 1.0f / (float)i;
    puts_("harmonic float="); putf(fsum, 6); nl();
    i64 conv = 0; for (int i = -500; i < 500; i++) { double v = i * 1.37; conv += (i64)v + (i64)(float)(v * 0.5) + (u32)(v > 0 ? v : -v); }
    puts_("conversions="); puti(conv); nl();
    return 0;
}
