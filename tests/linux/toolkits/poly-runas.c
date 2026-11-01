/*
 * poly-runas: run a program as another user of the toolkits' root disk
 * (ADR 0120), for programs that refuse to run as root (VLC). Written from
 * the man pages: passwd(5), setgroups(2), setgid(2), setuid(2), link(2),
 * unix(7), execvp(3), and the XDG Base Directory specification. Linked
 * statically.
 *
 *   poly-runas USER PROGRAM [ARGS...]
 *
 * It looks USER up in /etc/passwd, makes the user's home directory if
 * there is none and the user's runtime directory /run/user/<uid> (mode
 * 0700, the user's), and points HOME and the XDG directories at them.
 * The compositor's socket (WAYLAND_DISPLAY in the caller's runtime
 * directory, which stays private) gets a second name in the user's
 * runtime directory: a hard link, the same socket (unix(7): a socket is
 * found by its inode), made connectable for the user. Then it drops the
 * supplementary groups, sets the group and user IDs and runs PROGRAM.
 *
 * (Handing the program a connection as WAYLAND_SOCKET does not do:
 * VLC's Qt interface connects and disconnects once to see whether
 * Wayland is there, which uses up the handed connection.)
 *
 * Not in tests/linux/ itself: the diff runner compares those with real
 * Linux.
 */
#include <errno.h>
#include <grp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

/* The user's passwd(5) entry: 1 if found. */
static int lookup(const char *user, unsigned *uid, unsigned *gid, char *home, size_t home_len) {
    FILE *f = fopen("/etc/passwd", "r");
    if (!f) return 0;
    char line[512];
    int found = 0;
    while (!found && fgets(line, sizeof line, f)) {
        char *fields[7] = {0};
        char *p = line;
        for (int i = 0; i < 7 && p; i++) {
            fields[i] = p;
            p = strchr(p, ':');
            if (p) *p++ = 0;
        }
        if (fields[5] && strcmp(fields[0], user) == 0) {
            *uid = (unsigned)strtoul(fields[2], NULL, 10);
            *gid = (unsigned)strtoul(fields[3], NULL, 10);
            snprintf(home, home_len, "%s", fields[5]);
            found = 1;
        }
    }
    fclose(f);
    return found;
}

static void set_dir(const char *name, const char *home, const char *sub) {
    char path[512];
    snprintf(path, sizeof path, "%s/%s", home, sub);
    setenv(name, path, 1);
}

/* The compositor's socket under the same name in `runtime`; WAYLAND_DISPLAY
 * then names it there. */
static void share_display(const char *runtime) {
    const char *display = getenv("WAYLAND_DISPLAY");
    const char *old = getenv("XDG_RUNTIME_DIR");
    if (!display || !old || display[0] == '/' || strchr(display, '/')) return;
    char from[512], to[512];
    snprintf(from, sizeof from, "%s/%s", old, display);
    snprintf(to, sizeof to, "%s/%s", runtime, display);
    struct stat st;
    if (stat(from, &st) != 0 || !S_ISSOCK(st.st_mode)) return;
    unlink(to);
    /* Connecting needs write permission on the socket (unix(7)); the
     * caller's runtime directory keeps everyone else away from it. */
    if (link(from, to) != 0 || chmod(to, 0666) != 0) {
        perror("poly-runas: the compositor's socket");
    }
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: poly-runas USER PROGRAM [ARGS...]\n");
        return 2;
    }
    unsigned uid, gid;
    char home[256];
    if (!lookup(argv[1], &uid, &gid, home, sizeof home)) {
        fprintf(stderr, "poly-runas: no user %s in /etc/passwd\n", argv[1]);
        return 1;
    }
    /* The home directory, the user's, made the first time. */
    struct stat st;
    if (stat(home, &st) != 0) {
        if (mkdir(home, 0755) != 0 || chown(home, uid, gid) != 0) {
            perror("poly-runas: home directory");
            return 1;
        }
    }
    char runtime[64];
    snprintf(runtime, sizeof runtime, "/run/user/%u", uid);
    mkdir("/run/user", 0755);
    if (mkdir(runtime, 0700) != 0 && errno != EEXIST) {
        perror("poly-runas: mkdir");
        return 1;
    }
    if (chown(runtime, uid, gid) != 0 || chmod(runtime, 0700) != 0) {
        perror("poly-runas: runtime directory");
        return 1;
    }
    share_display(runtime);
    setenv("XDG_RUNTIME_DIR", runtime, 1);
    setenv("HOME", home, 1);
    setenv("USER", argv[1], 1);
    setenv("LOGNAME", argv[1], 1);
    set_dir("XDG_CONFIG_HOME", home, ".config");
    set_dir("XDG_DATA_HOME", home, ".local/share");
    set_dir("XDG_CACHE_HOME", home, ".cache");
    set_dir("XDG_STATE_HOME", home, ".local/state");
    if (setgroups(0, NULL) != 0 || setgid(gid) != 0 || setuid(uid) != 0) {
        perror("poly-runas: changing user");
        return 1;
    }
    if (chdir(home) != 0) {
        perror("poly-runas: chdir");
    }
    execvp(argv[2], argv + 2);
    fprintf(stderr, "poly-runas: %s: %s\n", argv[2], strerror(errno));
    return 127;
}
