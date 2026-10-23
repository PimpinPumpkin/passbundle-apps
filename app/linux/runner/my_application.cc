#include "my_application.h"

#include <flutter_linux/flutter_linux.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

#include "flutter/generated_plugin_registrant.h"

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
  // Where and how big the window is while it isn't maximized, and whether
  // it's maximized, kept for next time (save_place).
  gint width, height, x, y;
  gboolean placed, maximized;
  // A save waiting for the window to settle (a drag sends many moves).
  guint save_source;
  // Tells the app the computer is about to sleep, so it can lock first
  // (docs/core.md, "Locking by itself"): systemd-logind's PrepareForSleep.
  FlMethodChannel* computer;
  GDBusConnection* system_bus;
  guint sleep_signal;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

static void prepare_for_sleep_cb(GDBusConnection* connection,
                                 const gchar* sender, const gchar* path,
                                 const gchar* interface, const gchar* signal,
                                 GVariant* parameters, gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  gboolean going_to_sleep = FALSE;
  g_variant_get(parameters, "(b)", &going_to_sleep);
  if (going_to_sleep && self->computer != nullptr) {
    fl_method_channel_invoke_method(self->computer, "sleeping", nullptr,
                                    nullptr, nullptr, nullptr);
  }
}

static void listen_for_sleep(MyApplication* self, FlView* view) {
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  self->computer = fl_method_channel_new(
      fl_engine_get_binary_messenger(fl_view_get_engine(view)),
      "passbundle/computer", FL_METHOD_CODEC(codec));
  g_autoptr(GError) error = nullptr;
  self->system_bus = g_bus_get_sync(G_BUS_TYPE_SYSTEM, nullptr, &error);
  if (self->system_bus == nullptr) {
    // No system bus (a container, say): the app still locks on waking.
    return;
  }
  self->sleep_signal = g_dbus_connection_signal_subscribe(
      self->system_bus, "org.freedesktop.login1",
      "org.freedesktop.login1.Manager", "PrepareForSleep",
      "/org/freedesktop/login1", nullptr, G_DBUS_SIGNAL_FLAGS_NONE,
      prepare_for_sleep_cb, self, nullptr);
}

// The app's icon, from its bundle's data folder.
static void set_icon(GtkWindow* window) {
  g_autofree gchar* program = g_file_read_link("/proc/self/exe", nullptr);
  if (program == nullptr) {
    return;
  }
  g_autofree gchar* dir = g_path_get_dirname(program);
  g_autofree gchar* icon =
      g_build_filename(dir, "data", APPLICATION_ID ".png", nullptr);
  gtk_window_set_icon_from_file(window, icon, nullptr);
}

// The window is at least 400 by 560, as on the Mac.
static const gint kMinimumWidth = 400;
static const gint kMinimumHeight = 560;

// Where the window's place is kept between launches: next to the app's
// vault, in ~/.local/share/PassBundle (the core's folder).
static gchar* place_file() {
  return g_build_filename(g_get_user_data_dir(), "PassBundle", "window.ini",
                          nullptr);
}

// The main screen's work area (without panels), or a usual screen's.
static GdkRectangle work_area() {
  GdkRectangle area = {0, 0, 1440, 900};
  GdkDisplay* display = gdk_display_get_default();
  if (display == nullptr) {
    return area;
  }
  GdkMonitor* monitor = gdk_display_get_primary_monitor(display);
  if (monitor == nullptr && gdk_display_get_n_monitors(display) > 0) {
    monitor = gdk_display_get_monitor(display, 0);
  }
  if (monitor != nullptr) {
    gdk_monitor_get_workarea(monitor, &area);
  }
  return area;
}

// Whether the window can be put at a place: only X11 lets an app place its
// window; Wayland's compositor decides.
static gboolean can_place(GtkWindow* window) {
#ifdef GDK_WINDOWING_X11
  return GDK_IS_X11_DISPLAY(gtk_widget_get_display(GTK_WIDGET(window)));
#else
  return FALSE;
#endif
}

// The first time, 1200 by 800 (wide enough for the menu, the list, and an
// item side by side; the wide layout starts at 900), or as much of the main
// screen as there is; after that, where and how big the person left it, if
// that's still on a screen, and no bigger than it.
static void restore_place(MyApplication* self, GtkWindow* window) {
  GdkGeometry hints = {};
  hints.min_width = kMinimumWidth;
  hints.min_height = kMinimumHeight;
  gtk_window_set_geometry_hints(window, nullptr, &hints, GDK_HINT_MIN_SIZE);

  const GdkRectangle area = work_area();
  gint width = MIN(1200, area.width - 40);
  gint height = MIN(800, area.height - 40);
  gboolean maximized = FALSE;
  g_autofree gchar* path = place_file();
  g_autoptr(GKeyFile) kept = g_key_file_new();
  if (g_key_file_load_from_file(kept, path, G_KEY_FILE_NONE, nullptr)) {
    g_autoptr(GError) error = nullptr;
    const gint kept_width = g_key_file_get_integer(kept, "Window", "width", &error);
    const gint kept_height =
        error == nullptr ? g_key_file_get_integer(kept, "Window", "height", &error) : 0;
    if (error == nullptr && kept_width > 0 && kept_height > 0) {
      width = MIN(kept_width, area.width);
      height = MIN(kept_height, area.height);
      maximized = g_key_file_get_boolean(kept, "Window", "maximized", nullptr);
      g_autoptr(GError) no_place = nullptr;
      const gint x = g_key_file_get_integer(kept, "Window", "x", &no_place);
      const gint y =
          no_place == nullptr ? g_key_file_get_integer(kept, "Window", "y", &no_place) : 0;
      if (no_place == nullptr && can_place(window)) {
        // Only where the middle of the window is on one of the screens.
        const gint middle_x = x + width / 2, middle_y = y + height / 2;
        GdkMonitor* monitor = gdk_display_get_monitor_at_point(
            gtk_widget_get_display(GTK_WIDGET(window)), middle_x, middle_y);
        GdkRectangle there = {0, 0, 0, 0};
        if (monitor != nullptr) {
          gdk_monitor_get_geometry(monitor, &there);
        }
        if (middle_x >= there.x && middle_x < there.x + there.width &&
            middle_y >= there.y && middle_y < there.y + there.height) {
          gtk_window_move(window, x, y);
        }
      }
    }
  }
  width = MAX(width, kMinimumWidth);
  height = MAX(height, kMinimumHeight);
  gtk_window_set_default_size(window, width, height);
  self->width = width;
  self->height = height;
  if (maximized) {
    gtk_window_maximize(window);
  }
}

// Writes where and how big the window is, for next time.
static void save_place(MyApplication* self) {
  if (self->width <= 0 || self->height <= 0) {
    return;
  }
  g_autoptr(GKeyFile) kept = g_key_file_new();
  g_key_file_set_integer(kept, "Window", "width", self->width);
  g_key_file_set_integer(kept, "Window", "height", self->height);
  if (self->placed) {
    g_key_file_set_integer(kept, "Window", "x", self->x);
    g_key_file_set_integer(kept, "Window", "y", self->y);
  }
  g_key_file_set_boolean(kept, "Window", "maximized", self->maximized);
  g_autofree gchar* path = place_file();
  g_autofree gchar* dir = g_path_get_dirname(path);
  g_mkdir_with_parents(dir, 0700);
  g_key_file_save_to_file(kept, path, nullptr);
}

static gboolean save_soon_cb(gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  self->save_source = 0;
  save_place(self);
  return G_SOURCE_REMOVE;
}

// Saved half a second after the window settles, so a place is kept even if
// the app doesn't get to close its window (the session ends, say).
static void save_soon(MyApplication* self) {
  if (self->save_source != 0) {
    g_source_remove(self->save_source);
  }
  self->save_source = g_timeout_add(500, save_soon_cb, self);
}

// The window moved or changed size: kept, while it isn't maximized.
static gboolean configure_cb(GtkWidget* widget, GdkEventConfigure* event,
                             gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  GtkWindow* window = GTK_WINDOW(widget);
  if (!gtk_window_is_maximized(window)) {
    gtk_window_get_size(window, &self->width, &self->height);
    if (can_place(window)) {
      gtk_window_get_position(window, &self->x, &self->y);
      self->placed = TRUE;
    }
  }
  save_soon(self);
  return FALSE;
}

static gboolean window_state_cb(GtkWidget* widget, GdkEventWindowState* event,
                                gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  if ((event->changed_mask & GDK_WINDOW_STATE_MAXIMIZED) != 0) {
    self->maximized =
        (event->new_window_state & GDK_WINDOW_STATE_MAXIMIZED) != 0;
    save_soon(self);
  }
  return FALSE;
}

// The window is closing: kept now.
static gboolean delete_cb(GtkWidget* widget, GdkEvent* event,
                          gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  if (self->save_source != 0) {
    g_source_remove(self->save_source);
    self->save_source = 0;
  }
  save_place(self);
  return FALSE;
}

// Called when first Flutter frame received.
static void first_frame_cb(MyApplication* self, FlView* view) {
  gtk_widget_show(gtk_widget_get_toplevel(GTK_WIDGET(view)));
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));

  // Use a header bar when running in GNOME as this is the common style used
  // by applications and is the setup most users will be using (e.g. Ubuntu
  // desktop).
  // If running on X and not using GNOME then just use a traditional title bar
  // in case the window manager does more exotic layout, e.g. tiling.
  // If running on Wayland assume the header bar will work (may need changing
  // if future cases occur).
  gboolean use_header_bar = TRUE;
#ifdef GDK_WINDOWING_X11
  GdkScreen* screen = gtk_window_get_screen(window);
  if (GDK_IS_X11_SCREEN(screen)) {
    const gchar* wm_name = gdk_x11_screen_get_window_manager_name(screen);
    if (g_strcmp0(wm_name, "GNOME Shell") != 0) {
      use_header_bar = FALSE;
    }
  }
#endif
  if (use_header_bar) {
    GtkHeaderBar* header_bar = GTK_HEADER_BAR(gtk_header_bar_new());
    gtk_widget_show(GTK_WIDGET(header_bar));
    gtk_header_bar_set_title(header_bar, "PassBundle");
    gtk_header_bar_set_show_close_button(header_bar, TRUE);
    gtk_window_set_titlebar(window, GTK_WIDGET(header_bar));
  } else {
    gtk_window_set_title(window, "PassBundle");
  }

  restore_place(self, window);
  g_signal_connect(window, "configure-event", G_CALLBACK(configure_cb), self);
  g_signal_connect(window, "window-state-event", G_CALLBACK(window_state_cb),
                   self);
  g_signal_connect(window, "delete-event", G_CALLBACK(delete_cb), self);
  set_icon(window);

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  fl_dart_project_set_dart_entrypoint_arguments(
      project, self->dart_entrypoint_arguments);

  FlView* view = fl_view_new(project);
  GdkRGBA background_color;
  // Background defaults to black, override it here if necessary, e.g. #00000000
  // for transparent.
  gdk_rgba_parse(&background_color, "#000000");
  fl_view_set_background_color(view, &background_color);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  // Show the window when Flutter renders.
  // Requires the view to be realized so we can start rendering.
  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb),
                           self);
  gtk_widget_realize(GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));
  listen_for_sleep(self, view);

  gtk_widget_grab_focus(GTK_WIDGET(view));
}

// Implements GApplication::local_command_line.
static gboolean my_application_local_command_line(GApplication* application,
                                                  gchar*** arguments,
                                                  int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  // Strip out the first argument as it is the binary name.
  self->dart_entrypoint_arguments = g_strdupv(*arguments + 1);

  g_autoptr(GError) error = nullptr;
  if (!g_application_register(application, nullptr, &error)) {
    g_warning("Failed to register: %s", error->message);
    *exit_status = 1;
    return TRUE;
  }

  g_application_activate(application);
  *exit_status = 0;

  return TRUE;
}

// Implements GApplication::startup.
static void my_application_startup(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application startup.

  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
}

// Implements GApplication::shutdown.
static void my_application_shutdown(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application shutdown.

  G_APPLICATION_CLASS(my_application_parent_class)->shutdown(application);
}

// Implements GObject::dispose.
static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  if (self->save_source != 0) {
    g_source_remove(self->save_source);
    self->save_source = 0;
  }
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  if (self->system_bus != nullptr && self->sleep_signal != 0) {
    g_dbus_connection_signal_unsubscribe(self->system_bus, self->sleep_signal);
    self->sleep_signal = 0;
  }
  g_clear_object(&self->system_bus);
  g_clear_object(&self->computer);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->local_command_line =
      my_application_local_command_line;
  G_APPLICATION_CLASS(klass)->startup = my_application_startup;
  G_APPLICATION_CLASS(klass)->shutdown = my_application_shutdown;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {}

MyApplication* my_application_new() {
  // Set the program name to the application ID, which helps various systems
  // like GTK and desktop environments map this running application to its
  // corresponding .desktop file. This ensures better integration by allowing
  // the application to be recognized beyond its binary name.
  g_set_prgname(APPLICATION_ID);

  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID, "flags",
                                     G_APPLICATION_NON_UNIQUE, nullptr));
}
