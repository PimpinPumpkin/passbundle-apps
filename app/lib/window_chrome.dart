// The window's top edge on a computer, drawn by the app as 1Password draws
// its own: no system title bar over the app. On a Mac the traffic lights sit
// in the top left of the menu, lined up with the list's search; on Windows
// the app draws minimize, maximize, and close at the top right; on Linux it
// draws the buttons the desktop asks for, on the side it asks for them,
// unless the desktop can't take a window that draws its own (no compositor,
// a tiling window manager), which then keeps the system's title bar. Phones
// are untouched.
//
// The band along the top moves the window wherever nothing of the app's
// takes clicks there. Each runner (MainFlutterWindow.swift,
// win32_window.cpp, my_application.cc) decides a press before the app sees
// it, so after each frame the app tells it where the band is and where in
// it something does take clicks: a field, a button, an open menu, or a
// dialog's barrier, but only where a press would reach it.
import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import 'core.dart' show isPhone;
import 'theme.dart';

/// Who draws the top of the window.
enum FrameStyle {
  /// The system: on a phone, in a test, and on a Linux desktop that can't
  /// take a window drawing its own.
  system,

  /// The app, with the Mac's own traffic lights placed in it.
  mac,

  /// The app, with its own minimize, maximize, and close, which Windows
  /// treats as a caption's (snap layouts on maximize, too).
  windows,

  /// The app, with its own buttons, those the desktop asks for.
  linux,
}

/// A window's button the app draws (Windows and Linux).
enum WindowButton { minimize, maximize, close }

/// The wide layout's top band, where a title bar would be: the menu's top
/// (with a Mac's traffic lights in it), the list's search, and the item's
/// actions, a row 48 tall, 12 down (its middle [windowRow] down).
const double windowBand = 60;
const double windowRow = 36;

/// Everywhere else (signing in, unlocking, the narrow layout), a strip this
/// tall along the top, over the ground, with the window's buttons in it.
const double windowStrip = 32;

/// What the runner says about the window.
@immutable
class WindowFrame {
  const WindowFrame({
    this.style = FrameStyle.system,
    this.maximized = false,
    this.fullScreen = false,
    this.active = true,
    this.start = const [],
    this.end = const [],
  });

  /// From the runner's `frame` map; the system's frame for anything else.
  factory WindowFrame.from(Object? map) {
    if (map is! Map) return const WindowFrame();
    List<WindowButton> buttons(Object? names) => [
      if (names is List)
        for (final name in names)
          for (final b in WindowButton.values)
            if (b.name == name) b,
    ];
    final style = FrameStyle.values.firstWhere(
      (s) => s.name == map['style'],
      orElse: () => FrameStyle.system,
    );
    return WindowFrame(
      style: style,
      maximized: map['maximized'] == true,
      fullScreen: map['fullScreen'] == true,
      active: map['active'] != false,
      start: style == FrameStyle.linux ? buttons(map['start']) : const [],
      end: switch (style) {
        FrameStyle.windows => WindowButton.values,
        FrameStyle.linux => buttons(map['end']),
        _ => const [],
      },
    );
  }

  final FrameStyle style;
  final bool maximized;

  /// A Mac in full screen, where macOS hides the traffic lights.
  final bool fullScreen;

  /// The window has the focus: Windows and Linux dim their buttons without.
  final bool active;

  /// The buttons the app draws at the band's start and end, in order.
  final List<WindowButton> start;
  final List<WindowButton> end;

  @override
  bool operator ==(Object other) =>
      other is WindowFrame &&
      other.style == style &&
      other.maximized == maximized &&
      other.fullScreen == fullScreen &&
      other.active == active &&
      listEquals(other.start, start) &&
      listEquals(other.end, end);

  @override
  int get hashCode => Object.hash(
    style,
    maximized,
    fullScreen,
    active,
    Object.hashAll(start),
    Object.hashAll(end),
  );
}

/// The window, as the runner tells it, for the life of the app (the app
/// makes a new MaterialApp, and so a new [WindowChrome], each time it locks
/// or unlocks), and what the app tells the runner back.
class WindowFrameKeeper {
  WindowFrameKeeper._();

  static final instance = WindowFrameKeeper._();

  static const channel = MethodChannel('passbundle/window');

  final frame = ValueNotifier<WindowFrame>(const WindowFrame());

  /// The button the mouse is over and the one it holds down, on Windows,
  /// where the runner answers for the buttons and the app only draws them.
  final mouse = ValueNotifier<(WindowButton?, WindowButton?)>((null, null));

  /// What the runner was last told (`chrome`), so it hears only changes.
  Object? _told;

  /// Asks the runner, before the first frame, so the app opens as it stays.
  Future<void> load() async {
    if (isPhone ||
        !(Platform.isMacOS || Platform.isWindows || Platform.isLinux)) {
      return;
    }
    channel.setMethodCallHandler(_fromRunner);
    try {
      frame.value = WindowFrame.from(
        await channel
            .invokeMethod<Object?>('frame')
            .timeout(const Duration(seconds: 2)),
      );
    } on Object {
      // A runner without it: the system's frame.
      frame.value = const WindowFrame();
    }
  }

  /// Tests: the window as a runner would describe it.
  @visibleForTesting
  set debugFrame(WindowFrame value) {
    frame.value = value;
    mouse.value = (null, null);
    _told = null;
  }

  Future<Object?> _fromRunner(MethodCall call) async {
    switch (call.method) {
      case 'frame':
        frame.value = WindowFrame.from(call.arguments);
      case 'buttons':
        final arguments = call.arguments;
        WindowButton? named(Object? name) =>
            WindowButton.values.where((b) => b.name == name).firstOrNull;
        if (arguments is Map) {
          mouse.value = (
            named(arguments['hovered']),
            named(arguments['pressed']),
          );
        }
    }
    return null;
  }

  /// Minimizes, maximizes or restores, or closes the window, as its button.
  void press(WindowButton button) {
    channel.invokeMethod<void>(button.name).catchError((Object _) {});
  }

  /// Tells the runner where the band is and what in it takes clicks, when
  /// that changes.
  void tell(Map<String, Object?> chrome) {
    if (_deepEquals(_told, chrome)) return;
    _told = chrome;
    channel.invokeMethod<void>('chrome', chrome).catchError((Object _) {});
  }

  static bool _deepEquals(Object? a, Object? b) {
    if (a is List && b is List) {
      if (a.length != b.length) return false;
      for (var i = 0; i < a.length; i++) {
        if (!_deepEquals(a[i], b[i])) return false;
      }
      return true;
    }
    if (a is Map && b is Map) {
      if (a.length != b.length) return false;
      for (final key in a.keys) {
        if (!b.containsKey(key) || !_deepEquals(a[key], b[key])) return false;
      }
      return true;
    }
    return a == b;
  }
}

/// The app's pages, as its Navigator has them (main.dart gives each
/// MaterialApp one): the window's buttons go in the top page's focus, so Tab
/// goes round them with the page's own fields and buttons, but not with a
/// dialog's or a menu's, which keep Tab to themselves.
class WindowPages extends NavigatorObserver {
  final _routes = <Route<dynamic>>[];

  /// The top page's focus, under any dialog or menu over it.
  FocusScopeNode? get focus {
    final page = _routes.reversed
        .whereType<ModalRoute<dynamic>>()
        .where((r) => r is! PopupRoute)
        .firstOrNull
        ?.subtreeContext;
    return page == null || !page.mounted
        ? null
        : FocusScope.of(page, createDependency: false);
  }

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _routes.add(route);

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _routes.remove(route);

  @override
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _routes.remove(route);

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) {
    final at = oldRoute == null ? -1 : _routes.indexOf(oldRoute);
    if (at >= 0) _routes.removeAt(at);
    if (newRoute != null) {
      _routes.insert(at >= 0 ? at : _routes.length, newRoute);
    }
  }
}

/// How the window's top is laid out, for the screens under [WindowChrome].
@immutable
class WindowChromeData {
  const WindowChromeData({
    this.style = FrameStyle.system,
    this.integrated = false,
    this.startButtons = false,
    this.endInset = 0,
  });

  final FrameStyle style;

  /// The wide vault's band: no strip, and each pane's header in the band.
  final bool integrated;

  /// The window's buttons are at the band's start (a Mac's traffic lights,
  /// or Linux's on the left): the menu keeps its top for them.
  final bool startButtons;

  /// How far in from the band's end the window's buttons reach: the item's
  /// actions stop short of them.
  final double endInset;
}

/// The window's top, over the whole app (main.dart). [integrated] is the
/// wide vault's band; otherwise a strip along the top, which the screens
/// see as padding (MediaQuery), as they would a phone's status bar.
class WindowChrome extends StatefulWidget {
  const WindowChrome({
    super.key,
    required this.integrated,
    this.pages,
    required this.child,
  });

  final bool integrated;

  /// The pages under it, for Tab to reach the window's buttons.
  final WindowPages? pages;
  final Widget child;

  static WindowChromeData of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_ChromeScope>()?.data ??
      const WindowChromeData();

  @override
  State<WindowChrome> createState() => _WindowChromeState();
}

class _WindowChromeState extends State<WindowChrome> {
  final keeper = WindowFrameKeeper.instance;
  final keys = {for (final b in WindowButton.values) b: GlobalKey()};

  /// The top page's focus, where the window's buttons are for Tab.
  FocusScopeNode? pages;

  /// On Windows, the caption button the mouse has rested on, which shows
  /// its name: Windows answers for the mouse there, so the app's own
  /// tooltips never see it.
  WindowButton? tip;
  Timer? tipSoon;

  @override
  void initState() {
    super.initState();
    keeper.frame.addListener(changed);
    keeper.mouse.addListener(mouseMoved);
    SchedulerBinding.instance.addPostFrameCallback(afterFrame);
  }

  @override
  void dispose() {
    keeper.frame.removeListener(changed);
    keeper.mouse.removeListener(mouseMoved);
    tipSoon?.cancel();
    super.dispose();
  }

  /// As a tooltip waits (theme.dart), and goes as the mouse leaves or
  /// presses.
  void mouseMoved() {
    final (over, down) = keeper.mouse.value;
    tipSoon?.cancel();
    if (tip != null && (over != tip || down != null)) {
      setState(() => tip = null);
    }
    if (over == null || down != null || frame.style != FrameStyle.windows) {
      return;
    }
    final wait =
        Theme.of(context).tooltipTheme.waitDuration ??
        const Duration(milliseconds: 500);
    tipSoon = Timer(wait, () {
      if (mounted) setState(() => tip = over);
    });
  }

  void changed() => setState(() {});

  WindowFrame get frame => keeper.frame.value;

  /// Whether the app draws the top at all, now: not under the system's
  /// frame, and not in a Mac's full screen, where there's nothing to draw.
  bool get drawn =>
      frame.style != FrameStyle.system &&
      !(frame.style == FrameStyle.mac && frame.fullScreen);

  double get bandHeight => !drawn
      ? 0
      : widget.integrated
      ? windowBand
      : windowStrip;

  /// The middle of the window's buttons, from the top.
  double get row => widget.integrated ? windowRow : windowStrip / 2;

  /// A Mac's traffic lights: the close button's left edge, in from the
  /// window's (in the band, inside the menu's card), and their middle.
  Offset get lights =>
      widget.integrated ? const Offset(26, windowRow) : const Offset(12, 16);

  /// Linux's buttons: how far in from the window's edge, at either end.
  double get linuxInsetStart => widget.integrated ? 26 : 8;
  double get linuxInsetEnd => widget.integrated ? 20 : 8;

  double endInset() => switch (frame.style) {
    FrameStyle.windows => windowsButtonWidth * frame.end.length,
    FrameStyle.linux when frame.end.isNotEmpty =>
      linuxInsetEnd +
          (linuxButtonSize + linuxButtonGap) * frame.end.length -
          linuxButtonGap,
    _ => 0,
  };

  /// After each frame, while this is up: the runner hears where the band is
  /// and what takes clicks in it.
  void afterFrame(Duration _) {
    if (!mounted) return;
    if (frame.style != FrameStyle.system) {
      final focus = widget.pages?.focus;
      if (focus != pages) setState(() => pages = focus);
      report();
    }
    SchedulerBinding.instance.addPostFrameCallback(afterFrame);
  }

  void report() {
    final box = context.findRenderObject();
    if (box is! RenderBox || !box.hasSize || !box.attached) return;
    final width = box.size.width;
    final band = Rect.fromLTWH(0, 0, width, bandHeight);
    List<double> edges(Rect r) => [r.left, r.top, r.right, r.bottom];
    final buttons = <String, List<double>>{};
    for (final MapEntry(key: button, value: key) in keys.entries) {
      final b = key.currentContext?.findRenderObject();
      if (b is RenderBox && b.hasSize && b.attached) {
        buttons[button.name] = edges(
          MatrixUtils.transformRect(
            b.getTransformTo(null),
            Offset.zero & b.size,
          ),
        );
      }
    }
    keeper.tell({
      'band': edges(band),
      'holes': [
        for (final hole in band.isEmpty ? <Rect>[] : clickable(box, band))
          edges(hole),
      ],
      'buttons': buttons,
      if (frame.style == FrameStyle.mac && drawn)
        'lights': [lights.dx, lights.dy, bandHeight],
    });
  }

  /// Where in [band] a press reaches something that takes it: what has a
  /// mouse cursor of its own (buttons, fields, menus' entries, a dialog's
  /// barrier) or edits text, and that a press there would reach, not
  /// something drawn over it (the page over the barrier of its own route).
  List<Rect> clickable(RenderBox root, Rect band) {
    final view = View.of(context).viewId;
    bool reaches(Offset point, RenderObject target) {
      final result = HitTestResult();
      WidgetsBinding.instance.hitTestInView(result, point, view);
      return result.path.any((e) => identical(e.target, target));
    }

    final found = <Rect>[];
    // The way down to each node, and each one's box on the screen.
    final path = <(RenderObject, Rect?)>[];
    void visit(RenderObject node, Matrix4 transform, Rect clip) {
      if (node is RenderIgnorePointer && node.ignoring) return;
      if (node is RenderAbsorbPointer && node.absorbing) return;
      Rect? box;
      if (node is RenderBox) {
        if (!node.hasSize) return;
        box = MatrixUtils.transformRect(transform, Offset.zero & node.size);
        if (box.intersect(clip).isEmpty) return;
        if ((node is RenderMouseRegion &&
                node.cursor != MouseCursor.defer &&
                node.cursor != MouseCursor.uncontrolled) ||
            node is RenderEditable) {
          final hole = takes(node, box.intersect(clip), band, path, reaches);
          if (hole != null) found.add(hole);
          return;
        }
      }
      path.add((node, box));
      void child(RenderObject c) {
        final m = transform.clone();
        node.applyPaintTransform(c, m);
        var within = clip;
        final local = node.describeApproximatePaintClip(c);
        if (local != null) {
          within = within.intersect(
            MatrixUtils.transformRect(transform, local),
          );
        }
        if (!within.isEmpty) visit(c, m, within);
      }

      // What's shown: a covered page's entries and the other pages of a
      // stack of tabs aren't (they're onstage only for semantics), but what
      // is merely hidden from screen readers still takes clicks.
      if (node is RenderExcludeSemantics) {
        node.visitChildren(child);
      } else {
        node.visitChildrenForSemantics(child);
      }
      path.removeLast();
    }

    visit(root, root.getTransformTo(null), band);
    return found;
  }

  /// Where in [band] a press reaches [target], whose box there is [rect]:
  /// none if something drawn over it takes the press instead (a page over
  /// its own route's barrier, a dialog over the page); and as far as its
  /// tap target reaches past what it draws (Material pads a small button's
  /// out to 48 by 48), up through the boxes it's the only child of.
  static Rect? takes(
    RenderObject target,
    Rect rect,
    Rect band,
    List<(RenderObject, Rect?)> path,
    bool Function(Offset, RenderObject) reaches,
  ) {
    // A few places across what's wide (a barrier), where something on top
    // may cover part of it.
    final xs = rect.width < 100
        ? [rect.center.dx]
        : [
            for (final f in [0.1, 0.3, 0.5, 0.7, 0.9])
              rect.left + rect.width * f,
          ];
    if (!xs.any((x) => reaches(Offset(x, rect.center.dy), target))) {
      return null;
    }
    var hole = rect;
    for (var i = path.length - 1; i >= 0 && i >= path.length - 24; i--) {
      final (node, box) = path[i];
      if (box == null || node is! RenderObjectWithChildMixin) break;
      final wider = box.intersect(band);
      if (wider == hole) continue;
      if (!reaches(wider.topLeft + const Offset(0.5, 0.5), target) ||
          !reaches(wider.bottomRight - const Offset(0.5, 0.5), target)) {
        break;
      }
      hole = wider;
    }
    return hole;
  }

  @override
  Widget build(BuildContext context) {
    if (frame.style == FrameStyle.system) {
      return _ChromeScope(data: const WindowChromeData(), child: widget.child);
    }
    final media = MediaQuery.of(context);
    final strip = drawn && !widget.integrated ? windowStrip : 0.0;
    final data = WindowChromeData(
      style: frame.style,
      integrated: widget.integrated && drawn,
      startButtons:
          drawn && (frame.style == FrameStyle.mac || frame.start.isNotEmpty),
      endInset: drawn ? endInset() : 0,
    );
    final top = media.padding.top + strip;
    return _ChromeScope(
      data: data,
      child: Stack(
        fit: StackFit.passthrough,
        children: [
          MediaQuery(
            data: media.copyWith(
              padding: media.padding.copyWith(top: top),
              viewPadding: media.viewPadding.copyWith(
                top: media.viewPadding.top + strip,
              ),
            ),
            child: widget.child,
          ),
          if (drawn && frame.style == FrameStyle.windows)
            Positioned(top: 0, right: 0, child: buttons(frame.end)),
          if (drawn && frame.style == FrameStyle.windows && tip != null)
            Positioned(
              top: windowsButtonHeight + 4,
              // Under its button, in the window.
              right:
                  windowsButtonWidth *
                      (frame.end.length - 1 - frame.end.indexOf(tip!)) +
                  4,
              child: IgnorePointer(
                child: _Tip(windowButtonLabel(tip!, frame.maximized)),
              ),
            ),
          if (drawn && frame.style == FrameStyle.linux) ...[
            if (frame.start.isNotEmpty)
              Positioned(
                top: row - linuxButtonSize / 2,
                left: linuxInsetStart,
                child: buttons(frame.start),
              ),
            if (frame.end.isNotEmpty)
              Positioned(
                top: row - linuxButtonSize / 2,
                right: linuxInsetEnd,
                child: buttons(frame.end),
              ),
          ],
        ],
      ),
    );
  }

  Widget buttons(List<WindowButton> which) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      for (final (i, b) in which.indexed) ...[
        if (i > 0 && frame.style == FrameStyle.linux)
          const SizedBox(width: linuxButtonGap),
        _Button(key: keys[b], button: b, frame: frame, scope: pages),
      ],
    ],
  );
}

class _ChromeScope extends InheritedWidget {
  const _ChromeScope({required this.data, required super.child});

  final WindowChromeData data;

  @override
  bool updateShouldNotify(_ChromeScope old) =>
      old.data.style != data.style ||
      old.data.integrated != data.integrated ||
      old.data.startButtons != data.startButtons ||
      old.data.endInset != data.endInset;
}

/// What a window's button is called, to a screen reader and in its
/// tooltip.
String windowButtonLabel(WindowButton button, bool maximized) =>
    switch (button) {
      WindowButton.minimize => 'Minimize',
      WindowButton.maximize => maximized ? 'Restore' : 'Maximize',
      WindowButton.close => 'Close',
    };

/// A caption button's name under it, as the app's tooltips look.
class _Tip extends StatelessWidget {
  const _Tip(this.label);

  final String label;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return ExcludeSemantics(
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: scheme.inverseSurface,
          borderRadius: BorderRadius.circular(4),
        ),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
          child: Text(
            label,
            style: Theme.of(context).textTheme.bodySmall
                ?.copyWith(color: scheme.onInverseSurface),
          ),
        ),
      ),
    );
  }
}

/// A window's button the app draws: named for screen readers, reached with
/// Tab and pressed with Enter or Space, and lit while the mouse is over it
/// or holds it down. On Windows the runner says so: the mouse over a
/// button there is Windows' to answer for, so the app never sees it.
class _Button extends StatefulWidget {
  const _Button({
    super.key,
    required this.button,
    required this.frame,
    this.scope,
  });

  final WindowButton button;
  final WindowFrame frame;

  /// Where Tab finds it: the top page's focus (WindowPages).
  final FocusScopeNode? scope;

  @override
  State<_Button> createState() => _ButtonState();
}

class _ButtonState extends State<_Button> {
  bool hovered = false;
  bool pressed = false;
  bool focused = false;
  final focus = FocusNode(debugLabel: 'Window button');

  void press() => WindowFrameKeeper.instance.press(widget.button);

  @override
  void dispose() {
    focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final b = widget.button;
    final frame = widget.frame;
    final windows = frame.style == FrameStyle.windows;
    final keeper = WindowFrameKeeper.instance;
    return Semantics(
      container: true,
      button: true,
      label: windowButtonLabel(b, frame.maximized),
      onTap: press,
      child: Actions(
        actions: {
          ActivateIntent: CallbackAction<ActivateIntent>(
            onInvoke: (_) {
              press();
              return null;
            },
          ),
        },
        child: Focus(
          focusNode: focus,
          parentNode: widget.scope,
          includeSemantics: false,
          onFocusChange: (on) => setState(() => focused = on),
          child: MouseRegion(
            cursor: SystemMouseCursors.basic,
            onEnter: (_) => setState(() => hovered = true),
            onExit: (_) => setState(() => hovered = false),
            child: GestureDetector(
              onTapDown: (_) => setState(() => pressed = true),
              onTapCancel: () => setState(() => pressed = false),
              onTapUp: (_) => setState(() => pressed = false),
              onTap: press,
              child: ListenableBuilder(
                listenable: keeper.mouse,
                builder: (context, _) {
                  final (over, down) = keeper.mouse.value;
                  return CustomPaint(
                    size: windows
                        ? const Size(windowsButtonWidth, windowsButtonHeight)
                        : const Size.square(linuxButtonSize),
                    painter: _Glyph(
                      button: b,
                      windows: windows,
                      maximized: frame.maximized,
                      active: frame.active,
                      lit: hovered || over == b,
                      held: pressed || down == b,
                      // A ring only for the keyboard, as the app's other
                      // buttons have it.
                      focused:
                          focused &&
                          FocusManager.instance.highlightMode ==
                              FocusHighlightMode.traditional,
                      colors: context.colors,
                      focus: Theme.of(context).colorScheme.primary,
                    ),
                  );
                },
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Windows 11's caption buttons: 46 by 32, flush with the top.
const double windowsButtonWidth = 46;
const double windowsButtonHeight = 32;

/// GNOME's: round, 24 across, 10 apart.
const double linuxButtonSize = 24;
const double linuxButtonGap = 10;

/// A window button's look. Windows 11's: a quiet fill under the mouse, red
/// for close, and Windows' own glyphs, thin and square, in the app's text
/// color (dimmer while the window isn't active). GNOME's: a faint disc that
/// deepens under the mouse, with its symbolic glyphs.
class _Glyph extends CustomPainter {
  _Glyph({
    required this.button,
    required this.windows,
    required this.maximized,
    required this.active,
    required this.lit,
    required this.held,
    required this.focused,
    required this.colors,
    required this.focus,
  });

  /// Windows 11's close red, under the mouse and held down.
  static const red = Color(0xFFC42B1C);
  static const redHeld = Color(0xE6C42B1C);

  final WindowButton button;
  final bool windows;
  final bool maximized;
  final bool active;
  final bool lit;
  final bool held;
  final bool focused;
  final PassBundleColors colors;
  final Color focus;

  @override
  void paint(Canvas canvas, Size size) =>
      windows ? paintWindows(canvas, size) : paintLinux(canvas, size);

  void paintWindows(Canvas canvas, Size size) {
    final close = button == WindowButton.close;
    final fill = !lit && !held
        ? null
        : close
        ? (held ? redHeld : red)
        : colors.text.withValues(alpha: held ? 0.04 : 0.06);
    if (fill != null) {
      canvas.drawRect(Offset.zero & size, Paint()..color = fill);
    }
    final ink = close && (lit || held)
        ? Colors.white.withValues(alpha: held ? 0.7 : 1)
        : colors.text.withValues(
            alpha: !active
                ? 0.45
                : held
                ? 0.6
                : 1,
          );
    final pen = Paint()
      ..color = ink
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    // A 10 by 10 glyph in the middle, on the pixel grid.
    final c = Offset(
      (size.width / 2).floorToDouble() + 0.5,
      (size.height / 2).floorToDouble() + 0.5,
    );
    const r = Radius.circular(1);
    switch (button) {
      case WindowButton.minimize:
        canvas.drawLine(c + const Offset(-5, 0), c + const Offset(5, 0), pen);
      case WindowButton.maximize when maximized:
        // Restore: a square, and the corner of another behind it.
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTRB(c.dx - 5, c.dy - 3, c.dx + 3, c.dy + 5),
            r,
          ),
          pen,
        );
        canvas.drawPath(
          Path()
            ..moveTo(c.dx - 3, c.dy - 3)
            ..lineTo(c.dx - 3, c.dy - 4)
            ..arcToPoint(Offset(c.dx - 2, c.dy - 5), radius: r)
            ..lineTo(c.dx + 4, c.dy - 5)
            ..arcToPoint(Offset(c.dx + 5, c.dy - 4), radius: r)
            ..lineTo(c.dx + 5, c.dy + 2)
            ..arcToPoint(Offset(c.dx + 4, c.dy + 3), radius: r)
            ..lineTo(c.dx + 3, c.dy + 3),
          pen,
        );
      case WindowButton.maximize:
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTRB(c.dx - 5, c.dy - 5, c.dx + 5, c.dy + 5),
            r,
          ),
          pen,
        );
      case WindowButton.close:
        canvas.drawLine(c + const Offset(-5, -5), c + const Offset(5, 5), pen);
        canvas.drawLine(c + const Offset(5, -5), c + const Offset(-5, 5), pen);
    }
    if (focused) {
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          (Offset.zero & size).deflate(2),
          const Radius.circular(4),
        ),
        Paint()
          ..color = focus
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2,
      );
    }
  }

  void paintLinux(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    canvas.drawCircle(
      c,
      size.width / 2,
      Paint()
        ..color = colors.text.withValues(
          alpha: held
              ? 0.22
              : lit
              ? 0.14
              : 0.08,
        ),
    );
    final pen = Paint()
      ..color = colors.text.withValues(alpha: active ? 1 : 0.5)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5
      ..strokeCap = StrokeCap.round;
    const r = Radius.circular(1.5);
    switch (button) {
      case WindowButton.minimize:
        canvas.drawLine(c + const Offset(-4, 3), c + const Offset(4, 3), pen);
      case WindowButton.maximize when maximized:
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromCenter(
              center: c + const Offset(-1, 1),
              width: 7,
              height: 7,
            ),
            r,
          ),
          pen,
        );
        canvas.drawPath(
          Path()
            ..moveTo(c.dx - 1.5, c.dy - 3.5)
            ..lineTo(c.dx + 3.5, c.dy - 3.5)
            ..lineTo(c.dx + 3.5, c.dy + 1.5),
          pen,
        );
      case WindowButton.maximize:
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromCenter(center: c, width: 8, height: 8),
            r,
          ),
          pen,
        );
      case WindowButton.close:
        canvas.drawLine(c + const Offset(-4, -4), c + const Offset(4, 4), pen);
        canvas.drawLine(c + const Offset(4, -4), c + const Offset(-4, 4), pen);
    }
    if (focused) {
      canvas.drawCircle(
        c,
        size.width / 2 + 1,
        Paint()
          ..color = focus
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2,
      );
    }
  }

  @override
  bool shouldRepaint(_Glyph old) =>
      old.button != button ||
      old.windows != windows ||
      old.maximized != maximized ||
      old.active != active ||
      old.lit != lit ||
      old.held != held ||
      old.focused != focused ||
      old.colors != colors ||
      old.focus != focus;
}
