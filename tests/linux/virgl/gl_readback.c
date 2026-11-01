/*
 * gl-readback: OpenGL ES rendering checked pixel for pixel (ADR 0119).
 *
 * EGL on the surfaceless platform (Mesa picks its driver for the render
 * node: on Poly's virtio GPU with 3D, virgl), a GLES 2 context without a
 * surface, a 64x64 RGBA framebuffer object; it is cleared, a scissored
 * clear paints a rectangle, a shader draws a triangle over the right half,
 * and glReadPixels (the host's pixels coming back through the device)
 * must show exactly that. Prints the renderer and "gl-readback: ok".
 *
 * Built against the Khronos EGL and GLES headers, dynamically linked to
 * libEGL and libGLESv2.
 */
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define SIDE 64

static int failures;

static void check(int ok, const char *what) {
    if (!ok) {
        printf("gl-readback: FAIL %s\n", what);
        failures++;
    }
}

static GLuint shader(GLenum type, const char *src) {
    GLuint s = glCreateShader(type);
    glShaderSource(s, 1, &src, NULL);
    glCompileShader(s);
    GLint ok = 0;
    glGetShaderiv(s, GL_COMPILE_STATUS, &ok);
    if (!ok) {
        char log[512] = "";
        glGetShaderInfoLog(s, sizeof log, NULL, log);
        printf("gl-readback: shader: %s\n", log);
    }
    check(ok, "a shader compiles");
    return s;
}

/* The pixel at (x, y) of `px` (RGBA rows from the bottom). */
static const unsigned char *at(const unsigned char *px, int x, int y) {
    return px + (y * SIDE + x) * 4;
}

static int is(const unsigned char *p, int r, int g, int b) {
    return p[0] == r && p[1] == g && p[2] == b && p[3] == 255;
}

int main(void) {
    PFNEGLGETPLATFORMDISPLAYEXTPROC get_display =
        (PFNEGLGETPLATFORMDISPLAYEXTPROC)eglGetProcAddress("eglGetPlatformDisplayEXT");
    if (!get_display) {
        printf("gl-readback: FAIL no eglGetPlatformDisplayEXT\n");
        return 1;
    }
    EGLDisplay dpy = get_display(EGL_PLATFORM_SURFACELESS_MESA, EGL_DEFAULT_DISPLAY, NULL);
    EGLint major = 0, minor = 0;
    if (dpy == EGL_NO_DISPLAY || !eglInitialize(dpy, &major, &minor)) {
        printf("gl-readback: FAIL no EGL display (error %#x)\n", eglGetError());
        return 1;
    }
    eglBindAPI(EGL_OPENGL_ES_API);
    static const EGLint ctx_attrs[] = {EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE};
    EGLContext ctx = eglCreateContext(dpy, EGL_NO_CONFIG_KHR, EGL_NO_CONTEXT, ctx_attrs);
    if (ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, ctx)) {
        printf("gl-readback: FAIL no GLES 2 context (error %#x)\n", eglGetError());
        return 1;
    }
    printf("gl-readback: EGL %d.%d, renderer \"%s\", version \"%s\"\n", major, minor,
           (const char *)glGetString(GL_RENDERER), (const char *)glGetString(GL_VERSION));

    GLuint fbo, rb;
    glGenFramebuffers(1, &fbo);
    glBindFramebuffer(GL_FRAMEBUFFER, fbo);
    glGenRenderbuffers(1, &rb);
    glBindRenderbuffer(GL_RENDERBUFFER, rb);
    /* RGBA8 where the implementation has it (OES_rgb8_rgba8), else RGBA4:
       both hold 0 and 255 exactly, all this test draws. */
    glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, SIDE, SIDE);
    glRenderbufferStorage(GL_RENDERBUFFER, 0x8058 /* GL_RGBA8_OES */, SIDE, SIDE);
    (void)glGetError();
    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    check(glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE, "the framebuffer is complete");
    glViewport(0, 0, SIDE, SIDE);

    /* Red everywhere, then a green 16x16 square at (8, 8). */
    glClearColor(1, 0, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT);
    glEnable(GL_SCISSOR_TEST);
    glScissor(8, 8, 16, 16);
    glClearColor(0, 1, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT);
    glDisable(GL_SCISSOR_TEST);

    /* A blue rectangle over the right half (x >= 32), from a shader. */
    GLuint prog = glCreateProgram();
    glAttachShader(prog, shader(GL_VERTEX_SHADER, "attribute vec2 p; void main() { gl_Position = vec4(p, 0.0, 1.0); }"));
    glAttachShader(prog, shader(GL_FRAGMENT_SHADER,
                                "precision mediump float; void main() { gl_FragColor = vec4(0.0, 0.0, 1.0, 1.0); }"));
    glBindAttribLocation(prog, 0, "p");
    glLinkProgram(prog);
    GLint linked = 0;
    glGetProgramiv(prog, GL_LINK_STATUS, &linked);
    check(linked, "the program links");
    glUseProgram(prog);
    static const GLfloat quad[] = {0, -1, 1, -1, 0, 1, 1, 1};
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, quad);
    glEnableVertexAttribArray(0);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);

    unsigned char *px = calloc(SIDE * SIDE, 4);
    glReadPixels(0, 0, SIDE, SIDE, GL_RGBA, GL_UNSIGNED_BYTE, px);
    check(glGetError() == GL_NO_ERROR, "no GL error");
    check(is(at(px, 0, 0), 255, 0, 0), "red at (0, 0)");
    check(is(at(px, 31, 63), 255, 0, 0), "red at (31, 63)");
    check(is(at(px, 8, 8), 0, 255, 0), "green at (8, 8)");
    check(is(at(px, 23, 23), 0, 255, 0), "green at (23, 23)");
    check(is(at(px, 24, 24), 255, 0, 0), "red at (24, 24), past the square");
    check(is(at(px, 32, 0), 0, 0, 255), "blue at (32, 0)");
    check(is(at(px, 63, 63), 0, 0, 255), "blue at (63, 63)");
    int counts[3] = {0, 0, 0};
    for (int i = 0; i < SIDE * SIDE; i++) {
        const unsigned char *p = px + i * 4;
        counts[0] += is(p, 255, 0, 0);
        counts[1] += is(p, 0, 255, 0);
        counts[2] += is(p, 0, 0, 255);
    }
    printf("gl-readback: %d red, %d green, %d blue pixels\n", counts[0], counts[1], counts[2]);
    check(counts[0] == 32 * 64 - 16 * 16 && counts[1] == 16 * 16 && counts[2] == 32 * 64, "the pixel counts");
    free(px);
    eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    eglDestroyContext(dpy, ctx);
    eglTerminate(dpy);
    if (failures) {
        printf("gl-readback: %d failed\n", failures);
        return 1;
    }
    printf("gl-readback: ok\n");
    return 0;
}
