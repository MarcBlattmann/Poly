/*
 * gtk4-hello: a small GTK 4 app for Poly's desktop (ADR 0120), run
 * unchanged against the GTK 4 the root disk copies from the build image.
 * Written from GTK's public API documentation (docs.gtk.org/gtk4).
 *
 * A GtkApplication with one 360x240 window titled "Poly GTK 4" holding a
 * button coloured by CSS (GTK4_A, then GTK4_B once clicked). It reports
 * the window mapped, the first frame drawn, clicks and keys on standard
 * output for the boot test (tools/xtask/src/toolkits.rs); Q or the
 * window's close ends it.
 *
 * Not in tests/linux/ itself: the diff runner compares those with real
 * Linux.
 */
#include <gtk/gtk.h>
#include <stdio.h>

/* The button's two colours (toolkits.rs checks them on the screen). */
#define GTK4_A "#2a8a4a"
#define GTK4_B "#8a2a7a"

static const char *css =
    "button { background-image: none; border-radius: 0; border: none; box-shadow: none; }\n"
    "button.poly-a { background-color: " GTK4_A "; }\n"
    "button.poly-b { background-color: " GTK4_B "; }\n"
    "button label { color: white; }\n";

static GtkWidget *window;

static void on_clicked(GtkButton *b, gpointer data) {
    (void)data;
    GtkWidget *w = GTK_WIDGET(b);
    gboolean was_a = gtk_widget_has_css_class(w, "poly-a");
    gtk_widget_remove_css_class(w, was_a ? "poly-a" : "poly-b");
    gtk_widget_add_css_class(w, was_a ? "poly-b" : "poly-a");
    gtk_button_set_label(b, was_a ? "Clicked" : "Click me");
    printf("gtk4-hello: clicked\n");
}

static gboolean on_key(GtkEventControllerKey *c, guint keyval, guint keycode, GdkModifierType state,
                       gpointer data) {
    (void)c, (void)keycode, (void)state, (void)data;
    const char *name = gdk_keyval_name(keyval);
    printf("gtk4-hello: key %s\n", name ? name : "?");
    if (keyval == GDK_KEY_q || keyval == GDK_KEY_Q) {
        gtk_window_destroy(GTK_WINDOW(window));
    }
    return TRUE;
}

static void on_map(GtkWidget *w, gpointer data) {
    (void)data;
    printf("gtk4-hello: mapped\n");
    GdkSurface *s = gtk_native_get_surface(GTK_NATIVE(w));
    GskRenderer *r = gtk_native_get_renderer(GTK_NATIVE(w));
    printf("gtk4-hello: renderer %s, surface %dx%d\n", r ? G_OBJECT_TYPE_NAME(r) : "none",
           s ? gdk_surface_get_width(s) : 0, s ? gdk_surface_get_height(s) : 0);
}

static void on_frame(GdkFrameClock *clock, gpointer data) {
    (void)data;
    printf("gtk4-hello: drawn\n");
    g_signal_handlers_disconnect_by_func(clock, on_frame, data);
}

static void on_realize(GtkWidget *w, gpointer data) {
    (void)data;
    GdkFrameClock *clock = gtk_widget_get_frame_clock(w);
    if (clock) {
        g_signal_connect_after(clock, "after-paint", G_CALLBACK(on_frame), NULL);
    }
}

static void on_activate(GtkApplication *app, gpointer data) {
    (void)data;
    GtkCssProvider *provider = gtk_css_provider_new();
    gtk_css_provider_load_from_string(provider, css);
    gtk_style_context_add_provider_for_display(gdk_display_get_default(), GTK_STYLE_PROVIDER(provider),
                                               GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    window = gtk_application_window_new(app);
    gtk_window_set_title(GTK_WINDOW(window), "Poly GTK 4");
    gtk_window_set_default_size(GTK_WINDOW(window), 360, 240);
    GtkWidget *button = gtk_button_new_with_label("Click me");
    gtk_widget_add_css_class(button, "poly-a");
    g_signal_connect(button, "clicked", G_CALLBACK(on_clicked), NULL);
    gtk_window_set_child(GTK_WINDOW(window), button);
    GtkEventController *keys = gtk_event_controller_key_new();
    g_signal_connect(keys, "key-pressed", G_CALLBACK(on_key), NULL);
    gtk_widget_add_controller(window, keys);
    g_signal_connect(window, "map", G_CALLBACK(on_map), NULL);
    g_signal_connect_after(window, "realize", G_CALLBACK(on_realize), NULL);
    gtk_window_present(GTK_WINDOW(window));
    printf("gtk4-hello: GTK %u.%u.%u, display %s\n", gtk_get_major_version(), gtk_get_minor_version(),
           gtk_get_micro_version(), gdk_display_get_name(gdk_display_get_default()));
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    printf("gtk4-hello: starting\n");
    GtkApplication *app = gtk_application_new("org.poly.Gtk4Hello", G_APPLICATION_DEFAULT_FLAGS);
    g_signal_connect(app, "activate", G_CALLBACK(on_activate), NULL);
    int status = g_application_run(G_APPLICATION(app), argc, argv);
    g_object_unref(app);
    printf("gtk4-hello: closed, status %d\n", status);
    return status;
}
