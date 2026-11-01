/*
 * gtk3-hello: a small GTK 3 app for Poly's desktop (ADR 0120), run
 * unchanged against the GTK 3 the root disk copies from the build image.
 * Written from GTK's public API documentation (docs.gtk.org/gtk3).
 *
 * A GtkApplication with one 360x240 window titled "Poly GTK 3" holding a
 * button. The button's colour comes from a CSS provider (GTK3_A, then
 * GTK3_B once clicked), so a screenshot shows whether the click arrived.
 * It reports what happens on standard output, one line each, for the boot
 * test (tools/xtask/src/toolkits.rs): the window mapped and drawn, the
 * button clicked, keys pressed; Q or the window's close ends it.
 *
 * Not in tests/linux/ itself: the diff runner compares those with real
 * Linux.
 */
#include <gtk/gtk.h>
#include <stdio.h>

/* The button's two colours (toolkits.rs checks them on the screen). */
#define GTK3_A "#2a6fb0"
#define GTK3_B "#b03a2a"

static const char *css =
    "button { background-image: none; border-radius: 0; border: none; box-shadow: none; }\n"
    "button.poly-a { background-color: " GTK3_A "; }\n"
    "button.poly-b { background-color: " GTK3_B "; }\n"
    "button label { color: white; }\n";

static gboolean drawn_once;

static gboolean on_draw(GtkWidget *w, cairo_t *cr, gpointer data) {
    (void)w, (void)cr, (void)data;
    if (!drawn_once) {
        drawn_once = TRUE;
        printf("gtk3-hello: drawn\n");
    }
    return FALSE;
}

static void on_clicked(GtkButton *b, gpointer data) {
    (void)data;
    GtkStyleContext *ctx = gtk_widget_get_style_context(GTK_WIDGET(b));
    gboolean was_a = gtk_style_context_has_class(ctx, "poly-a");
    gtk_style_context_remove_class(ctx, was_a ? "poly-a" : "poly-b");
    gtk_style_context_add_class(ctx, was_a ? "poly-b" : "poly-a");
    gtk_button_set_label(b, was_a ? "Clicked" : "Click me");
    printf("gtk3-hello: clicked\n");
}

static gboolean on_key(GtkWidget *w, GdkEventKey *ev, gpointer data) {
    (void)data;
    const char *name = gdk_keyval_name(ev->keyval);
    printf("gtk3-hello: key %s\n", name ? name : "?");
    if (ev->keyval == GDK_KEY_q || ev->keyval == GDK_KEY_Q) {
        gtk_widget_destroy(w);
    }
    return TRUE;
}

static void on_map(GtkWidget *w, gpointer data) {
    (void)w, (void)data;
    printf("gtk3-hello: mapped\n");
}

static void on_activate(GtkApplication *app, gpointer data) {
    (void)data;
    GtkCssProvider *provider = gtk_css_provider_new();
    gtk_css_provider_load_from_data(provider, css, -1, NULL);
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(provider),
                                              GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    GtkWidget *win = gtk_application_window_new(app);
    gtk_window_set_title(GTK_WINDOW(win), "Poly GTK 3");
    gtk_window_set_default_size(GTK_WINDOW(win), 360, 240);
    GtkWidget *button = gtk_button_new_with_label("Click me");
    gtk_style_context_add_class(gtk_widget_get_style_context(button), "poly-a");
    g_signal_connect(button, "clicked", G_CALLBACK(on_clicked), NULL);
    gtk_container_add(GTK_CONTAINER(win), button);
    g_signal_connect(win, "key-press-event", G_CALLBACK(on_key), NULL);
    g_signal_connect(win, "map", G_CALLBACK(on_map), NULL);
    g_signal_connect_after(win, "draw", G_CALLBACK(on_draw), NULL);
    gtk_widget_show_all(win);
    printf("gtk3-hello: GTK %u.%u.%u, display %s\n", gtk_get_major_version(), gtk_get_minor_version(),
           gtk_get_micro_version(), gdk_display_get_name(gdk_display_get_default()));
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IOLBF, 0);
    printf("gtk3-hello: starting\n");
    GtkApplication *app = gtk_application_new("org.poly.Gtk3Hello", G_APPLICATION_DEFAULT_FLAGS);
    g_signal_connect(app, "activate", G_CALLBACK(on_activate), NULL);
    int status = g_application_run(G_APPLICATION(app), argc, argv);
    g_object_unref(app);
    printf("gtk3-hello: closed, status %d\n", status);
    return status;
}
