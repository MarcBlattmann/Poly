/* Error messages of a filesystem context. When fsconfig rejects a
 * parameter, the context descriptor gets a message that read(2) returns,
 * one whole message per read, oldest first, "e " (error) followed by text and
 * a newline; read gives ENODATA when none is left, and a successful
 * fsconfig adds none. Observed texts:
 *   unknown parameter of tmpfs:  "e tmpfs: Unknown parameter 'NAME'\n"
 *   bad value of a tmpfs option: "e tmpfs: Bad value for 'NAME'\n"
 *   a second "source":           "e Multiple sources\n"
 *   a path given as "source":    "e Non-string source\n"
 *   creating squashfs without a source: "e No source specified\n"
 * A buffer smaller than the next message is not enough: read gives
 * EMSGSIZE and keeps the message.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fsconfig_messages"

static void body(const char *base) {
    (void)base;
    char buf[256];
    i64 fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    expect("no_message_yet", a_read(fc, buf, sizeof buf), -ENODATA);

    expect("unknown_param", pm_fsconfig(fc, FSCONFIG_SET_STRING, "bogus", "1", 0), -EINVAL);
    check("msg_unknown_param", pm_msg_is(fc, "e tmpfs: Unknown parameter 'bogus'\n"), 0);
    expect("drained", a_read(fc, buf, sizeof buf), -ENODATA);

    expect("unknown_flag", pm_fsconfig(fc, FSCONFIG_SET_FLAG, "bogusflag", 0, 0), -EINVAL);
    check("msg_unknown_flag", pm_msg_is(fc, "e tmpfs: Unknown parameter 'bogusflag'\n"), 0);

    expect("bad_size", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "abc", 0), -EINVAL);
    check("msg_bad_size", pm_msg_is(fc, "e tmpfs: Bad value for 'size'\n"), 0);
    expect("bad_mode", pm_fsconfig(fc, FSCONFIG_SET_STRING, "mode", "999", 0), -EINVAL);
    check("msg_bad_mode", pm_msg_is(fc, "e tmpfs: Bad value for 'mode'\n"), 0);

    /* messages queue up and come out oldest first */
    expect("bad_a", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "x", 0), -EINVAL);
    expect("bad_b", pm_fsconfig(fc, FSCONFIG_SET_STRING, "nr_inodes", "y", 0), -EINVAL);
    check("queue_first", pm_msg_is(fc, "e tmpfs: Bad value for 'size'\n"), 0);
    check("queue_second", pm_msg_is(fc, "e tmpfs: Bad value for 'nr_inodes'\n"), 0);
    expect("queue_empty", a_read(fc, buf, sizeof buf), -ENODATA);

    /* a flag where a value is needed */
    expect("flag_for_size", pm_fsconfig(fc, FSCONFIG_SET_FLAG, "size", 0, 0), -EINVAL);
    check("msg_flag_for_size", pm_msg_is(fc, "e tmpfs: Bad value for 'size'\n"), 0);

    /* a small buffer */
    expect("bad_c", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "z", 0), -EINVAL);
    expect("small_buffer", a_read(fc, buf, 4), -EMSGSIZE);
    check("message_kept", pm_msg_is(fc, "e tmpfs: Bad value for 'size'\n"), 0);

    /* a good call adds nothing */
    expect("good", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "1m", 0), 0);
    expect("no_message_after_good", a_read(fc, buf, sizeof buf), -ENODATA);

    /* "source" rules */
    i64 f2 = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    expect("source_first", pm_fsconfig(f2, FSCONFIG_SET_STRING, "source", "mytmp", 0), 0);
    expect("source_second", pm_fsconfig(f2, FSCONFIG_SET_STRING, "source", "other", 0), -EINVAL);
    check("msg_multiple_sources", pm_msg_is(f2, "e Multiple sources\n"), 0);
    i64 f3 = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    expect("source_as_path", pm_fsconfig(f3, FSCONFIG_SET_PATH, "source", "/tmp", AT_FDCWD), -EINVAL);
    check("msg_non_string_source", pm_msg_is(f3, "e Non-string source\n"), 0);

    /* a filesystem that needs a source */
    i64 f4 = pm_fsopen("squashfs", FSOPEN_CLOEXEC);
    expect("squashfs_without_source", pm_fsconfig(f4, FSCONFIG_CMD_CREATE, 0, 0, 0), -EINVAL);
    check("msg_no_source", pm_msg_is(f4, "e No source specified\n"), 0);
    expect("failed_create_poisons", pm_fsconfig(f4, FSCONFIG_SET_STRING, "source", "/nonexistent", 0), -EBUSY);

    /* overlay checks its directories when they are given */
    i64 f5 = pm_fsopen("overlay", FSOPEN_CLOEXEC);
    expect("overlay_lowerdir_missing", pm_fsconfig(f5, FSCONFIG_SET_STRING, "lowerdir", "/nonexistent-dir", 0),
           -ENOENT);
    expect("overlay_bad_redirect", pm_fsconfig(f5, FSCONFIG_SET_STRING, "redirect_dir", "bogus", 0), -EINVAL);
    check("msg_overlay_bad_value", pm_msg_is(f5, "e overlay: Bad value for 'redirect_dir'\n"), 0);
}

int test_main(void) { return pm_main(BASE, body); }
