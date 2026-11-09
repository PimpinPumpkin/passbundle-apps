#include "my_application.h"

#include <flutter_linux/flutter_linux.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

#include <math.h>
#include <string.h>

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
  // The app draws the window's top (app/lib/window_chrome.dart), unless
  // the desktop can't take a window that draws its own (own_frame).
  GtkWindow* window;
  FlView* view;
  gboolean own_frame;
  FlMethodChannel* chrome;
  // The app's band along the top, and what in it takes clicks, in its
  // logical pixels from its view's top left: [left, top, right, bottom].
  gdouble band[4];
  GArray* holes;
  // A press on the band, until it moves far enough to move the window, or
  // lets go.
  gboolean pressing;
  guint press_button;
  gdouble press_x, press_y;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

static void tell_the_app(MyApplication* self);

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
    // Maximize turns into restore.
    tell_the_app(self);
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

// Whether the app draws the window's top: where the desktop's windows can
// have their own (a compositor, for their shadow and edges), and not on a
// tiling window manager, whose windows have no title bars to speak of. With
// GTK_CSD=0, the person has asked for the system's.
static gboolean draws_own_frame(GtkWindow* window) {
  if (g_strcmp0(g_getenv("GTK_CSD"), "0") == 0) {
    return FALSE;
  }
  static const gchar* const tiling[] = {
      "i3",     "sway",  "bspwm", "awesome",  "xmonad",   "lg3d",
      "dwm",    "qtile", "herbstluftwm", "hyprland", "river", "niri",
      "spectrwm", "leftwm", "ratpoison", "stumpwm", nullptr};
  auto is_tiling = [](const gchar* names) -> gboolean {
    if (names == nullptr) {
      return FALSE;
    }
    g_auto(GStrv) each = g_strsplit(names, ":", -1);
    for (gchar** name = each; *name != nullptr; name++) {
      g_autofree gchar* lower = g_ascii_strdown(*name, -1);
      for (const gchar* const* t = tiling; *t != nullptr; t++) {
        if (g_strcmp0(lower, *t) == 0) {
          return TRUE;
        }
      }
    }
    return FALSE;
  };
  if (is_tiling(g_getenv("XDG_CURRENT_DESKTOP")) ||
      is_tiling(g_getenv("XDG_SESSION_DESKTOP"))) {
    return FALSE;
  }
#ifdef GDK_WINDOWING_X11
  GdkScreen* screen = gtk_window_get_screen(window);
  if (GDK_IS_X11_SCREEN(screen)) {
    return gdk_screen_is_composited(screen) &&
           !is_tiling(gdk_x11_screen_get_window_manager_name(screen));
  }
#endif
  return TRUE;
}

// The window's buttons the desktop asks for (gtk-decoration-layout), at the
// start (before the colon) and the end: of minimize, maximize, and close.
static FlValue* buttons_at(const gchar* layout, gboolean end) {
  FlValue* buttons = fl_value_new_list();
  if (layout == nullptr) {
    return buttons;
  }
  g_auto(GStrv) sides = g_strsplit(layout, ":", 2);
  const gchar* side = end ? (sides[0] != nullptr ? sides[1] : nullptr)
                          : sides[0];
  if (side == nullptr) {
    return buttons;
  }
  g_auto(GStrv) names = g_strsplit(side, ",", -1);
  for (gchar** name = names; *name != nullptr; name++) {
    g_strstrip(*name);
    if (g_strcmp0(*name, "minimize") == 0 ||
        g_strcmp0(*name, "maximize") == 0 || g_strcmp0(*name, "close") == 0) {
      fl_value_append_take(buttons, fl_value_new_string(*name));
    }
  }
  return buttons;
}

// The window, as the app hears it: who draws its top, and its buttons.
static FlValue* frame_for_the_app(MyApplication* self) {
  FlValue* frame = fl_value_new_map();
  fl_value_set_string_take(
      frame, "style", fl_value_new_string(self->own_frame ? "linux" : "system"));
  fl_value_set_string_take(
      frame, "maximized",
      fl_value_new_bool(gtk_window_is_maximized(self->window)));
  fl_value_set_string_take(
      frame, "active", fl_value_new_bool(gtk_window_is_active(self->window)));
  g_autofree gchar* layout = nullptr;
  g_object_get(gtk_widget_get_settings(GTK_WIDGET(self->window)),
               "gtk-decoration-layout", &layout, nullptr);
  fl_value_set_string_take(frame, "start", buttons_at(layout, FALSE));
  fl_value_set_string_take(frame, "end", buttons_at(layout, TRUE));
  return frame;
}

static void tell_the_app(MyApplication* self) {
  if (self->chrome != nullptr && self->window != nullptr) {
    g_autoptr(FlValue) frame = frame_for_the_app(self);
    fl_method_channel_invoke_method(self->chrome, "frame", frame, nullptr,
                                    nullptr, nullptr);
  }
}

static void frame_changed_cb(GObject* object, GParamSpec* spec,
                             gpointer user_data) {
  tell_the_app(MY_APPLICATION(user_data));
}

static void toggle_maximized(GtkWindow* window) {
  if (gtk_window_is_maximized(window)) {
    gtk_window_unmaximize(window);
  } else {
    gtk_window_maximize(window);
  }
}

// [left, top, right, bottom] from the app, into |edges|.
static gboolean edges_of(FlValue* value, gdouble edges[4]) {
  if (value == nullptr || fl_value_get_type(value) != FL_VALUE_TYPE_LIST ||
      fl_value_get_length(value) != 4) {
    return FALSE;
  }
  for (size_t i = 0; i < 4; i++) {
    FlValue* edge = fl_value_get_list_value(value, i);
    switch (fl_value_get_type(edge)) {
      case FL_VALUE_TYPE_FLOAT:
        edges[i] = fl_value_get_float(edge);
        break;
      case FL_VALUE_TYPE_INT:
        edges[i] = fl_value_get_int(edge);
        break;
      default:
        return FALSE;
    }
  }
  return TRUE;
}

static void chrome_call_cb(FlMethodChannel* channel, FlMethodCall* call,
                           gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  const gchar* method = fl_method_call_get_name(call);
  FlValue* arguments = fl_method_call_get_args(call);
  gboolean map = arguments != nullptr &&
                 fl_value_get_type(arguments) == FL_VALUE_TYPE_MAP;
  g_autoptr(FlValue) answer = nullptr;
  if (g_strcmp0(method, "frame") == 0) {
    answer = frame_for_the_app(self);
  } else if (g_strcmp0(method, "chrome") == 0 && map) {
    if (!edges_of(fl_value_lookup_string(arguments, "band"), self->band)) {
      memset(self->band, 0, sizeof(self->band));
    }
    g_array_set_size(self->holes, 0);
    FlValue* holes = fl_value_lookup_string(arguments, "holes");
    if (holes != nullptr && fl_value_get_type(holes) == FL_VALUE_TYPE_LIST) {
      for (size_t i = 0; i < fl_value_get_length(holes); i++) {
        gdouble hole[4];
        if (edges_of(fl_value_get_list_value(holes, i), hole)) {
          g_array_append_vals(self->holes, hole, 4);
        }
      }
    }
  } else if (g_strcmp0(method, "minimize") == 0) {
    gtk_window_iconify(self->window);
  } else if (g_strcmp0(method, "maximize") == 0) {
    toggle_maximized(self->window);
  } else if (g_strcmp0(method, "close") == 0) {
    gtk_window_close(self->window);
  } else if (g_strcmp0(method, "look") == 0 && map) {
    // The system's title bar, where it has it, and GTK's menus, as the
    // app is: dark or light.
    FlValue* dark = fl_value_lookup_string(arguments, "dark");
    if (dark != nullptr && fl_value_get_type(dark) == FL_VALUE_TYPE_BOOL) {
      g_object_set(gtk_settings_get_default(),
                   "gtk-application-prefer-dark-theme",
                   fl_value_get_bool(dark), nullptr);
    }
  } else {
    fl_method_call_respond_not_implemented(call, nullptr);
    return;
  }
  fl_method_call_respond_success(call, answer, nullptr);
}

// Whether a point on the screen is on the app's band, where nothing of the
// app's takes clicks.
static gboolean on_the_band(MyApplication* self, gdouble x_root,
                            gdouble y_root) {
  if (self->band[2] <= self->band[0] || self->band[3] <= self->band[1]) {
    return FALSE;
  }
  GtkWidget* window = GTK_WIDGET(self->window);
  GdkWindow* toplevel = gtk_widget_get_window(window);
  gint origin_x = 0, origin_y = 0, view_x = 0, view_y = 0;
  if (toplevel == nullptr ||
      !gtk_widget_translate_coordinates(GTK_WIDGET(self->view), window, 0, 0,
                                        &view_x, &view_y)) {
    return FALSE;
  }
  gdk_window_get_origin(toplevel, &origin_x, &origin_y);
  const gdouble x = x_root - origin_x - view_x;
  const gdouble y = y_root - origin_y - view_y;
  auto inside = [x, y](const gdouble* box) -> gboolean {
    return x >= box[0] && x < box[2] && y >= box[1] && y < box[3];
  };
  if (!inside(self->band)) {
    return FALSE;
  }
  const gdouble* holes = reinterpret_cast<gdouble*>(self->holes->data);
  for (guint i = 0; i + 3 < self->holes->len; i += 4) {
    if (inside(holes + i)) {
      return FALSE;
    }
  }
  return TRUE;
}

// What a click on a title bar does, as the desktop says (GTK's
// gtk-titlebar-double-click, -middle-click, and -right-click).
static void as_a_title_bar(MyApplication* self, const gchar* setting,
                           GdkEvent* event) {
  g_autofree gchar* action = nullptr;
  g_object_get(gtk_widget_get_settings(GTK_WIDGET(self->window)), setting,
               &action, nullptr);
  GdkWindow* toplevel = gtk_widget_get_window(GTK_WIDGET(self->window));
  if (action == nullptr || g_strcmp0(action, "none") == 0) {
    return;
  } else if (g_str_has_prefix(action, "toggle-maximize")) {
    toggle_maximized(self->window);
  } else if (g_strcmp0(action, "minimize") == 0) {
    gtk_window_iconify(self->window);
  } else if (g_strcmp0(action, "lower") == 0) {
    gdk_window_lower(toplevel);
  } else if (g_strcmp0(action, "menu") == 0) {
    gdk_window_show_window_menu(toplevel, event);
  }
}

// Presses on the band, before the app sees them: one moves the window once
// it moves as far as a drag (as GTK's own title bars do), a double click
// does what the desktop says, and the other buttons too.
static gboolean took_for_the_band(MyApplication* self, GdkEvent* event) {
  if (event->any.window == nullptr ||
      gdk_window_get_toplevel(event->any.window) !=
          gtk_widget_get_window(GTK_WIDGET(self->window))) {
    return FALSE;
  }
  switch (event->type) {
    case GDK_BUTTON_PRESS:
    case GDK_2BUTTON_PRESS:
    case GDK_3BUTTON_PRESS: {
      if (!on_the_band(self, event->button.x_root, event->button.y_root)) {
        return FALSE;
      }
      const guint button = event->button.button;
      if (event->type == GDK_2BUTTON_PRESS && button == GDK_BUTTON_PRIMARY) {
        self->pressing = FALSE;
        as_a_title_bar(self, "gtk-titlebar-double-click", event);
      } else if (event->type != GDK_BUTTON_PRESS) {
        // The third of a triple click: nothing.
      } else if (button == GDK_BUTTON_PRIMARY) {
        self->pressing = TRUE;
        self->press_button = button;
        self->press_x = event->button.x_root;
        self->press_y = event->button.y_root;
      } else if (button == GDK_BUTTON_MIDDLE) {
        as_a_title_bar(self, "gtk-titlebar-middle-click", event);
      } else if (button == GDK_BUTTON_SECONDARY) {
        as_a_title_bar(self, "gtk-titlebar-right-click", event);
      }
      return TRUE;
    }
    case GDK_MOTION_NOTIFY: {
      if (!self->pressing) {
        return FALSE;
      }
      gint threshold = 8;
      g_object_get(gtk_widget_get_settings(GTK_WIDGET(self->window)),
                   "gtk-dnd-drag-threshold", &threshold, nullptr);
      if (hypot(event->motion.x_root - self->press_x,
                event->motion.y_root - self->press_y) >= threshold) {
        self->pressing = FALSE;
        gtk_window_begin_move_drag(self->window, self->press_button,
                                   static_cast<gint>(self->press_x),
                                   static_cast<gint>(self->press_y),
                                   event->motion.time);
      }
      return TRUE;
    }
    case GDK_BUTTON_RELEASE:
      if (!self->pressing) {
        return FALSE;
      }
      self->pressing = FALSE;
      return TRUE;
    default:
      return FALSE;
  }
}

static void handle_event(GdkEvent* event, gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  if (self->own_frame && self->window != nullptr && self->view != nullptr &&
      took_for_the_band(self, event)) {
    return;
  }
  gtk_main_do_event(event);
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

  self->window = window;
  self->own_frame = draws_own_frame(window);
  // The app draws the window's top (app/lib/window_chrome.dart): a title
  // bar of nothing, so GTK draws only the window's edges (its shadow, and
  // resizing from them), and the app the rest. Where the desktop can't
  // take that, the system's title bar, as before: a header bar on GNOME,
  // and the window manager's own elsewhere (tiling, say).
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
  if (self->own_frame) {
    GtkWidget* nothing = gtk_drawing_area_new();
    gtk_widget_set_no_show_all(nothing, TRUE);
    gtk_window_set_titlebar(window, nothing);
    gtk_window_set_title(window, "PassBundle");
    g_signal_connect(window, "notify::is-active", G_CALLBACK(frame_changed_cb),
                     self);
    g_signal_connect(gtk_widget_get_settings(GTK_WIDGET(window)),
                     "notify::gtk-decoration-layout",
                     G_CALLBACK(frame_changed_cb), self);
    gdk_event_handler_set(handle_event, self, nullptr);
  } else if (use_header_bar) {
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
  // At least 400 by 560 (kMinimumWidth): asked of the app's view, so GTK
  // adds what it draws around it (its shadow, when the app draws the
  // window's top) to what it asks the window manager for.
  gtk_widget_set_size_request(GTK_WIDGET(view), kMinimumWidth,
                              kMinimumHeight);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  // Show the window when Flutter renders.
  // Requires the view to be realized so we can start rendering.
  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb),
                           self);
  gtk_widget_realize(GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));
  listen_for_sleep(self, view);

  self->view = view;
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  self->chrome = fl_method_channel_new(
      fl_engine_get_binary_messenger(fl_view_get_engine(view)),
      "passbundle/window", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(self->chrome, chrome_call_cb,
                                            self, nullptr);

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
  g_clear_object(&self->chrome);
  g_clear_pointer(&self->holes, g_array_unref);
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

static void my_application_init(MyApplication* self) {
  self->holes = g_array_new(FALSE, FALSE, sizeof(gdouble));
}

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
