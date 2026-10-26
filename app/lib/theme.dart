// PassBundle's own look, the same on every platform: brand colors, Google
// Sans Flex (bundled, so no system font changes it), and our own shapes. Light
// and dark, or following the system. The design language: a calm ground;
// surfaces that rise in three steps (resting, lifted, floating); and a
// jewel-toned color for each kind of item. In the light, the ground is solid
// and the surfaces white. Dark mode is graphite glass, as the web vault and
// the browser extension have it (passbundle-clients, brand/look.css): a
// neutral graphite ground with faint jewel washes, and translucent white
// cards with a light edge, never green; emerald is only the accent.
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'glyphs.dart';

const brandGreen = Color(0xFF1F6F5C);

/// The typeface, everywhere: one variable font, whose weight axis follows
/// each style's FontWeight (pubspec.yaml).
const passBundleFontFamily = 'Google Sans Flex';

/// A faint jewel-toned light on the dark ground, brightest at [center] (a
/// corner or an edge of the screen) and gone [width] and [height] of the
/// screen's width and height away from it. Drawn as one two-color radial
/// gradient, which every renderer draws alike.
class Wash {
  const Wash(this.center, this.width, this.height, this.color);

  final Alignment center;
  final double width;
  final double height;
  final Color color;
}

/// How a surface sits at one of the depths: its fill; in the dark, the
/// hairline [edge] and the [highlight] along its top that make translucent
/// glass read as a surface; and its [shadows].
class Depth {
  const Depth(this.color, {this.edge, this.highlight, this.shadows = const []});

  final Color color;
  final Color? edge;
  final Color? highlight;
  final List<BoxShadow> shadows;

  /// It, with [radius] corners, and [ring] (a chosen row's) in place of
  /// its edge: in the light, a solid surface with the ring just outside it;
  /// in the dark, glass.
  Decoration decoration(BorderRadius radius, {Color? ring}) =>
      edge == null && highlight == null
      ? BoxDecoration(
          color: color,
          borderRadius: radius,
          boxShadow: [
            if (ring != null) BoxShadow(color: ring, spreadRadius: 1),
            ...shadows,
          ],
        )
      : Glass(
          color: color,
          radius: radius,
          edge: ring ?? edge,
          highlight: highlight,
          shadows: shadows,
        );
}

/// Every color of one mode, in one place: the light, and the dark sets.
/// Dark mode is [graphiteGlass]; [plainGraphite] and [trueBlack] are the
/// other dark sets the owner can choose, by changing [dark] alone.
class PassBundleColors extends ThemeExtension<PassBundleColors> {
  const PassBundleColors({
    required this.brightness,
    required this.ground,
    required this.washes,
    required this.text,
    required this.heading,
    required this.muted,
    required this.accent,
    required this.onAccent,
    required this.error,
    required this.pane,
    required this.edge,
    required this.restingLight,
    required this.restingShadows,
    required this.raised,
    required this.liftedEdge,
    required this.liftedLight,
    required this.liftedShadows,
    required this.lift,
    required this.selected,
    required this.selectedRing,
    required this.tint,
    required this.line,
    required this.divider,
    required this.outline,
    required this.field,
    required this.fieldEdge,
    required this.button,
    required this.buttonEdge,
    required this.well,
    required this.wellEdge,
    required this.chip,
    required this.chipEdge,
    required this.chipText,
    required this.canvas,
    required this.dialog,
    required this.dialogEdge,
    required this.dialogGround,
    required this.menu,
    required this.menuEdge,
    required this.sunken,
    required this.inverse,
    required this.onInverse,
    required this.letterTile,
    required this.letter,
    required this.siteTile,
    required this.siteTileEdge,
    required this.wordmarkPass,
    required this.wordmarkBundle,
  });

  /// A dark set: the colors that make it ([ground], [washes], and the
  /// surfaces), and those every dark set shares.
  const PassBundleColors._dark({
    required this.ground,
    this.washes = const [],
    required this.pane,
    this.edge,
    this.restingLight,
    required this.raised,
    this.liftedEdge,
    this.liftedLight,
    required this.line,
    required this.divider,
    required this.field,
    required this.fieldEdge,
    required this.button,
    required this.buttonEdge,
    required this.well,
    required this.wellEdge,
    required this.chip,
    this.chipEdge,
    required this.canvas,
    required this.dialog,
    this.dialogEdge,
    required this.menu,
    this.menuEdge,
    this.sunken = const Color(0x3D000000),
  }) : brightness = Brightness.dark,
       text = const Color(0xFFEEF0F3),
       heading = const Color(0xFFC9CED6),
       muted = const Color(0xFFA9AFB8),
       accent = const Color(0xFF6FD4B4),
       onAccent = const Color(0xFF06231B),
       // Light enough for 4.5:1 on any card.
       error = const Color(0xFFFF8A85),
       restingShadows = const [],
       liftedShadows = const [
         BoxShadow(
           color: Color(0x66000000),
           offset: Offset(0, 12),
           blurRadius: 32,
         ),
       ],
       lift = const [
         BoxShadow(
           color: Color(0x59000000),
           offset: Offset(0, 6),
           blurRadius: 16,
         ),
       ],
       selected = const Color(0x1F6FD4B4),
       selectedRing = const Color(0x596FD4B4),
       tint = const Color(0x1F6FD4B4),
       outline = const Color(0xFF7A818C),
       chipText = const Color(0xFFD0D4DB),
       inverse = const Color(0xFFEEF0F3),
       onInverse = const Color(0xFF1C1F25),
       letterTile = const Color(0xFF1C2A4A),
       letter = const Color(0xFF9DB4F2),
       siteTile = const Color(0xFFF2F4F7),
       siteTileEdge = const Color(0xFFF2F4F7),
       dialogGround = dialog,
       wordmarkPass = const Color(0xFFE7EFEC),
       wordmarkBundle = const Color(0xFF6FD4B4);

  final Brightness brightness;

  /// The calm ground everything sits on, and in the dark its washes.
  final Color ground;
  final List<Wash> washes;

  /// Text: body, a section's heading over the ground, and what's secondary.
  final Color text;
  final Color heading;
  final Color muted;

  /// What you act on: primary buttons, links, focus, selection, switches;
  /// and what's written on it.
  final Color accent;
  final Color onAccent;

  /// Errors, where the seed's own (null) isn't the one.
  final Color? error;

  /// Resting on the ground: lists, cards, panels ([pane]), with an [edge]
  /// and the [restingLight] along their top in the dark.
  final Color pane;
  final Color? edge;
  final Color? restingLight;
  final List<BoxShadow> restingShadows;

  /// Lifted off it: the chosen item, and its panel on a computer.
  final Color raised;
  final Color? liftedEdge;
  final Color? liftedLight;
  final List<BoxShadow> liftedShadows;

  /// What lifts a tile off what's under it: an item's big tile.
  final List<BoxShadow> lift;

  /// The selection: a tab's or a choice's fill, and a chosen row's ring.
  final Color selected;
  final Color selectedRing;

  /// The quiet fill behind a copy button and the menu's chosen entry, in the
  /// accent's family.
  final Color tint;

  /// Lines around things, and between the rows of a card.
  final Color line;
  final Color divider;

  /// Controls' outlines: a switch that's off, a choice's border.
  final Color outline;

  /// Text fields, and secondary buttons.
  final Color field;
  final Color fieldEdge;
  final Color button;
  final Color buttonEdge;

  /// Set into a lifted card: an item's details on a computer.
  final Color well;
  final Color wellEdge;

  /// A quiet pill: where an item is, in the list.
  final Color chip;
  final Color? chipEdge;
  final Color chipText;

  /// What floats over the rest, solid so nothing shows through: what's
  /// drawn on Material's canvas (a dropdown's list), dialogs and sheets,
  /// and menus.
  final Color canvas;
  final Color dialog;
  final Color? dialogEdge;

  /// What a dialog of cards (Settings) stands on: in the light the ground,
  /// since a dialog is as white as its cards.
  final Color dialogGround;
  final Color menu;
  final Color? menuEdge;

  /// Set into a card: a generated password, a switch's track.
  final Color sunken;

  /// A snack bar's and a tooltip's, where the seed's own (null) isn't.
  final Color? inverse;
  final Color? onInverse;

  /// A login's first letter where its website has no icon, and the white
  /// tile a website's icon sits on.
  final Color letterTile;
  final Color letter;
  final Color siteTile;
  final Color siteTileEdge;

  /// The logo's name, "pass" and "bundle" (brand/wordmark.py).
  final Color wordmarkPass;
  final Color wordmarkBundle;

  /// The three depths: lists and panels; the chosen item and its panel;
  /// what floats (menus, sheets, and dialogs) is Material's, in the theme.
  Depth get resting =>
      Depth(pane, edge: edge, highlight: restingLight, shadows: restingShadows);
  Depth get lifted => Depth(
    raised,
    edge: liftedEdge,
    highlight: liftedLight,
    shadows: liftedShadows,
  );

  /// The phone's status and navigation bars, and the iPhone's status bar:
  /// see-through, over the ground, with icons that show on it.
  SystemUiOverlayStyle get bars {
    final dark = brightness == Brightness.dark;
    final icons = dark ? Brightness.light : Brightness.dark;
    return SystemUiOverlayStyle(
      statusBarColor: Colors.transparent,
      statusBarBrightness: brightness,
      statusBarIconBrightness: icons,
      systemStatusBarContrastEnforced: false,
      systemNavigationBarColor: Colors.transparent,
      systemNavigationBarDividerColor: Colors.transparent,
      systemNavigationBarIconBrightness: icons,
      systemNavigationBarContrastEnforced: false,
    );
  }

  static const light = PassBundleColors(
    brightness: Brightness.light,
    ground: Color(0xFFEEF2F0),
    washes: [],
    text: Color(0xFF14201C),
    heading: Color(0xFF3C4A45),
    muted: Color(0xFF52615C),
    accent: brandGreen,
    onAccent: Color(0xFFFFFFFF),
    error: null,
    pane: Color(0xFFFFFFFF),
    edge: null,
    restingLight: null,
    restingShadows: [
      BoxShadow(color: Color(0x0F10201B), offset: Offset(0, 1), blurRadius: 2),
      BoxShadow(color: Color(0x0A10201B), offset: Offset(0, 1), blurRadius: 1),
    ],
    raised: Color(0xFFFFFFFF),
    liftedEdge: null,
    liftedLight: null,
    liftedShadows: _liftedLight,
    lift: _liftedLight,
    selected: Color(0xFFD6EBE3),
    selectedRing: Color(0xFFCFE3DA),
    tint: Color(0xFFE7F1ED),
    line: Color(0xFFDCE5E1),
    divider: Color(0xFFEDF2F0),
    outline: Color(0xFFCBD7D2),
    field: Color(0xFFFFFFFF),
    fieldEdge: Color(0xFFDCE5E1),
    button: Color(0xFFFFFFFF),
    buttonEdge: Color(0xFFCBD7D2),
    well: Color(0xFFF7FAF9),
    wellEdge: Color(0xFFDCE5E1),
    chip: Color(0xFFEEF2F0),
    chipEdge: null,
    chipText: Color(0xFF3C4A45),
    canvas: Color(0xFFFFFFFF),
    dialog: Color(0xFFFFFFFF),
    dialogEdge: null,
    dialogGround: Color(0xFFEEF2F0),
    menu: Color(0xFFFFFFFF),
    menuEdge: null,
    sunken: Color(0xFFE7EEEB),
    inverse: null,
    onInverse: null,
    letterTile: Color(0xFFE3E9F8),
    letter: Color(0xFF1D3C99),
    siteTile: Color(0xFFFFFFFF),
    siteTileEdge: Color(0xFFDCE5E1),
    wordmarkPass: Color(0xFF1B3A33),
    wordmarkBundle: Color(0xFF0E7A5A),
  );

  static const _liftedLight = [
    BoxShadow(color: Color(0x1410201B), offset: Offset(0, 4), blurRadius: 12),
    BoxShadow(color: Color(0x0F10201B), offset: Offset(0, 1), blurRadius: 3),
  ];

  /// Dark mode.
  static const dark = graphiteGlass;

  /// Graphite glass: a #111318 ground with faint sapphire, amethyst, and
  /// emerald washes, as the web vault has them (toned down from the mockup's
  /// 0.28, 0.22, and 0.18 so secondary text keeps 4.5:1 where they're
  /// brightest); cards translucent white with a light edge, and no blur
  /// (it's slow on Android, and draws differently on each platform).
  /// Dialogs, sheets, and menus are solid: a dialog is what a card over the
  /// bare ground looks like.
  static const graphiteGlass = PassBundleColors._dark(
    ground: Color(0xFF111318),
    washes: [
      // Sapphire, at the top left: radial-gradient(75% 55% at 0% 0%,
      // rgb(36 72 184 / 0.24), transparent 72%).
      Wash(Alignment.topLeft, 0.54, 0.396, Color(0x3D2448B8)),
      // Amethyst, at the top right: (60% 40% at 100% 10%, 0.14, 70%).
      Wash(Alignment(1, -0.8), 0.42, 0.28, Color(0x246B30B8)),
      // Emerald, at the bottom: (80% 50% at 50% 100%, 0.12, 70%).
      Wash(Alignment.bottomCenter, 0.56, 0.35, Color(0x1F0E7A5A)),
    ],
    pane: Color(0x0EFFFFFF),
    edge: Color(0x1CFFFFFF),
    restingLight: Color(0x0AFFFFFF),
    raised: Color(0x16FFFFFF),
    liftedEdge: Color(0x1FFFFFFF),
    liftedLight: Color(0x0FFFFFFF),
    line: Color(0x17FFFFFF),
    divider: Color(0x12FFFFFF),
    field: Color(0x0FFFFFFF),
    fieldEdge: Color(0x17FFFFFF),
    button: Color(0x0FFFFFFF),
    buttonEdge: Color(0x29FFFFFF),
    well: Color(0x0AFFFFFF),
    wellEdge: Color(0x14FFFFFF),
    chip: Color(0x1AFFFFFF),
    chipEdge: Color(0x1FFFFFFF),
    canvas: Color(0xFF1C1F25),
    // The ground with the cards' white over it, at 0.07.
    dialog: Color(0xFF222428),
    dialogEdge: Color(0x29FFFFFF),
    menu: Color(0xFF1C1F25),
    menuEdge: Color(0x1AFFFFFF),
  );

  /// Plain graphite: solid cards on a #121417 ground, with only a faint
  /// emerald glow at the top.
  static const plainGraphite = PassBundleColors._dark(
    ground: Color(0xFF121417),
    washes: [Wash(Alignment.topCenter, 0.56, 0.35, Color(0x1F0E7A5A))],
    pane: Color(0xFF1A1D21),
    edge: Color(0xFF262A30),
    raised: Color(0xFF1F2227),
    liftedEdge: Color(0xFF2E333A),
    line: Color(0xFF262A30),
    divider: Color(0xFF22262B),
    field: Color(0xFF1A1D21),
    fieldEdge: Color(0xFF262A30),
    button: Color(0xFF1A1D21),
    buttonEdge: Color(0xFF343940),
    well: Color(0xFF16181C),
    wellEdge: Color(0xFF262A30),
    chip: Color(0xFF23262B),
    canvas: Color(0xFF1A1D21),
    dialog: Color(0xFF1A1D21),
    dialogEdge: Color(0xFF2E333A),
    menu: Color(0xFF1A1D21),
    menuEdge: Color(0xFF2E333A),
    sunken: Color(0xFF121417),
  );

  /// True black: cards a step off a #000000 ground, no washes.
  static const trueBlack = PassBundleColors._dark(
    ground: Color(0xFF000000),
    pane: Color(0xFF111214),
    edge: Color(0xFF1E2023),
    raised: Color(0xFF16181B),
    liftedEdge: Color(0xFF26292D),
    line: Color(0xFF1E2023),
    divider: Color(0xFF1A1C1F),
    field: Color(0xFF111214),
    fieldEdge: Color(0xFF1E2023),
    button: Color(0xFF111214),
    buttonEdge: Color(0xFF2A2D31),
    well: Color(0xFF0A0B0C),
    wellEdge: Color(0xFF1E2023),
    chip: Color(0xFF1A1C1F),
    canvas: Color(0xFF111214),
    dialog: Color(0xFF111214),
    dialogEdge: Color(0xFF26292D),
    menu: Color(0xFF111214),
    menuEdge: Color(0xFF26292D),
    sunken: Color(0xFF000000),
  );

  @override
  PassBundleColors copyWith() => this;

  @override
  PassBundleColors lerp(ThemeExtension<PassBundleColors>? other, double t) =>
      t < 0.5 || other is! PassBundleColors ? this : other;
}

/// A surface of glass: a translucent fill, a hairline edge, light along its
/// top, and shadows that fall only outside it, since under translucent glass
/// they'd show through. No blur behind it.
class Glass extends Decoration {
  const Glass({
    required this.color,
    required this.radius,
    this.edge,
    this.highlight,
    this.shadows = const [],
  });

  final Color color;
  final BorderRadius radius;
  final Color? edge;
  final Color? highlight;
  final List<BoxShadow> shadows;

  @override
  BoxPainter createBoxPainter([VoidCallback? onChanged]) => _GlassPainter(this);

  @override
  bool operator ==(Object other) =>
      other is Glass &&
      other.color == color &&
      other.radius == radius &&
      other.edge == edge &&
      other.highlight == highlight &&
      _sameShadows(other.shadows, shadows);

  @override
  int get hashCode =>
      Object.hash(color, radius, edge, highlight, Object.hashAll(shadows));

  static bool _sameShadows(List<BoxShadow> a, List<BoxShadow> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }
}

class _GlassPainter extends BoxPainter {
  _GlassPainter(this.glass);

  final Glass glass;

  @override
  void paint(Canvas canvas, Offset offset, ImageConfiguration configuration) {
    final size = configuration.size;
    if (size == null || size.isEmpty) return;
    final rect = offset & size;
    final rrect = glass.radius.toRRect(rect);
    if (glass.shadows.isNotEmpty) {
      var reach = 0.0;
      for (final s in glass.shadows) {
        final r = s.blurRadius * 2 + s.spreadRadius + s.offset.distance;
        if (r > reach) reach = r;
      }
      canvas.save();
      canvas.clipPath(
        Path()
          ..fillType = PathFillType.evenOdd
          ..addRect(rect.inflate(reach + 1))
          ..addRRect(rrect),
      );
      for (final s in glass.shadows) {
        canvas.drawRRect(
          rrect.shift(s.offset).inflate(s.spreadRadius),
          s.toPaint(),
        );
      }
      canvas.restore();
    }
    canvas.drawRRect(rrect, Paint()..color = glass.color);
    final highlight = glass.highlight;
    if (highlight != null) {
      // One pixel of light inside the top edge, following its corners.
      canvas.save();
      canvas.clipRRect(rrect);
      canvas.drawPath(
        Path()
          ..fillType = PathFillType.evenOdd
          ..addRRect(rrect)
          ..addRRect(rrect.shift(const Offset(0, 1))),
        Paint()..color = highlight,
      );
      canvas.restore();
    }
    final edge = glass.edge;
    if (edge != null) {
      canvas.drawRRect(
        rrect.deflate(0.5),
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1
          ..color = edge,
      );
    }
  }
}

/// The ground, with its washes in the dark: behind each page of the app
/// (the dark theme's page transitions put it there), and wherever else the
/// ground shows.
class Ground extends StatelessWidget {
  const Ground({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Stack(
      fit: StackFit.passthrough,
      children: [
        Positioned.fill(
          child: RepaintBoundary(
            child: CustomPaint(painter: _GroundPainter(c.ground, c.washes)),
          ),
        ),
        child,
      ],
    );
  }
}

class _GroundPainter extends CustomPainter {
  _GroundPainter(this.ground, this.washes);

  final Color ground;
  final List<Wash> washes;

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    canvas.drawRect(rect, Paint()..color = ground);
    if (washes.isEmpty || size.isEmpty) return;
    canvas.save();
    canvas.clipRect(rect);
    for (final w in washes) {
      final center = w.center.withinRect(rect);
      final rx = w.width * size.width;
      final ry = w.height * size.height;
      if (rx <= 0 || ry <= 0) continue;
      // A unit circle, stretched into the wash's ellipse: two colors, the
      // second the first with nothing of it left.
      canvas.save();
      canvas.translate(center.dx, center.dy);
      canvas.scale(rx, ry);
      canvas.drawRect(
        const Rect.fromLTRB(-1, -1, 1, 1),
        Paint()
          ..shader = ui.Gradient.radial(Offset.zero, 1, [
            w.color,
            w.color.withAlpha(0),
          ]),
      );
      canvas.restore();
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_GroundPainter old) =>
      old.ground != ground || !identical(old.washes, washes);
}

/// The system's bars over the app, matching it: on a phone, the status and
/// navigation bars see-through over the ground (Android draws the app under
/// them on every version, main.dart), with icons that show on it; on a
/// computer, the window's frame light or dark as the app is, and in the dark
/// its title bar in the ground's color, where the runner can set it
/// (MainFlutterWindow.swift, flutter_window.cpp).
class SystemBars extends StatefulWidget {
  const SystemBars({super.key, required this.child});

  final Widget child;

  @override
  State<SystemBars> createState() => _SystemBarsState();
}

class _SystemBarsState extends State<SystemBars> {
  static const window = MethodChannel('passbundle/window');

  /// What the window was last told: whether it's dark, and the ground.
  (bool, int)? told;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (!Platform.isMacOS && !Platform.isWindows) return;
    final c = context.colors;
    final look = (
      c.brightness == Brightness.dark,
      c.ground.toARGB32() & 0xFFFFFF,
    );
    if (look == told) return;
    told = look;
    window
        .invokeMethod<void>('look', {'dark': look.$1, 'ground': look.$2})
        // A runner without it (a test) keeps its own frame.
        .catchError((Object _) {});
  }

  @override
  Widget build(BuildContext context) => AnnotatedRegion<SystemUiOverlayStyle>(
    value: context.colors.bars,
    child: widget.child,
  );
}

/// The platform's own page transitions, each page on the ground: in the
/// dark, where pages' own backgrounds are see-through so the washes show.
class _OnTheGround extends PageTransitionsTheme {
  const _OnTheGround();

  @override
  Widget buildTransitions<T>(
    PageRoute<T> route,
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
    Widget child,
  ) => super.buildTransitions(
    route,
    context,
    animation,
    secondaryAnimation,
    Ground(child: child),
  );
}

/// A jewel tone and a glyph: a kind of item's, or a place's in the menu.
/// Deep enough for a white glyph on it, light and dark, and a step lighter in
/// the dark, as the web vault has them (passbundle-clients, categories.ts).
class Jewel {
  const Jewel(this.glyph, this.light, this.dark);

  /// SVG path data on a 24 by 24 grid (glyphs.dart).
  final String glyph;
  final Color light;
  final Color dark;

  Color color(Brightness brightness) =>
      brightness == Brightness.dark ? dark : light;

  /// Emerald, the logo's color: everything.
  static const allItems = Jewel(
    Glyphs.grid,
    Color(0xFF0E7A5A),
    Color(0xFF0E7A5A),
  );

  /// Gold.
  static const favorites = Jewel(
    Glyphs.star,
    Color(0xFFB98310),
    Color(0xFFB98310),
  );

  /// A bank's emerald: Health, as the web vault's menu has it.
  static const health = Jewel(
    Glyphs.health,
    Color(0xFF0E7A47),
    Color(0xFF128752),
  );
}

/// The kinds of item, each with its jewel, and what the screens call one and
/// many.
class ItemKind extends Jewel {
  const ItemKind(
    this.kind,
    this.one,
    this.many,
    super.glyph,
    super.light,
    super.dark,
  );

  /// As the core names it.
  final String kind;
  final String one;
  final String many;

  /// Sapphire.
  static const login = ItemKind(
    'login',
    'Login',
    'Logins',
    Glyphs.key,
    Color(0xFF2448B8),
    Color(0xFF2E55C9),
  );

  /// Amethyst.
  static const card = ItemKind(
    'card',
    'Card',
    'Cards',
    Glyphs.card,
    Color(0xFF6B30B8),
    Color(0xFF7A42CC),
  );

  /// Turquoise.
  static const identity = ItemKind(
    'identity',
    'Identity',
    'Identities',
    Glyphs.identity,
    Color(0xFF08788A),
    Color(0xFF0A8599),
  );

  /// Topaz.
  static const note = ItemKind(
    'note',
    'Note',
    'Notes',
    Glyphs.note,
    Color(0xFFA9600C),
    Color(0xFFB4690F),
  );

  /// Onyx.
  static const sshKey = ItemKind(
    'sshKey',
    'SSH key',
    'SSH keys',
    Glyphs.terminal,
    Color(0xFF353B4A),
    Color(0xFF4A5266),
  );

  /// Upstream's newer kinds, which the web vault makes where the server
  /// offers them: emerald for a bank account, an identity's turquoise for
  /// the others.
  static const bankAccount = ItemKind(
    'bankAccount',
    'Bank account',
    'Bank accounts',
    Glyphs.bank,
    Color(0xFF0E7A47),
    Color(0xFF128752),
  );

  static const driversLicense = ItemKind(
    'driversLicense',
    'Driver\'s license',
    'Driver\'s licenses',
    Glyphs.identity,
    Color(0xFF08788A),
    Color(0xFF0A8599),
  );

  static const passport = ItemKind(
    'passport',
    'Passport',
    'Passports',
    Glyphs.identity,
    Color(0xFF08788A),
    Color(0xFF0A8599),
  );

  /// In the order the vault lists them.
  static const all = [
    login,
    card,
    identity,
    note,
    sshKey,
    bankAccount,
    driversLicense,
    passport,
  ];

  static ItemKind of(String kind) =>
      all.where((k) => k.kind == kind).firstOrNull ?? login;
}

/// The web vault's categories (passbundle-clients, categories.ts): each kind
/// of item's look, and the templates that bring their own (Wi-Fi
/// aquamarine, servers garnet, licenses ruby), as the core names them
/// (core/src/templates.rs). An item's tile shows its category's jewel.
abstract final class Categories {
  static const _looks = <String, Jewel>{
    'login': ItemKind.login,
    'note': ItemKind.note,
    'card': ItemKind.card,
    'identity': ItemKind.identity,
    'sshKey': ItemKind.sshKey,
    'bank': ItemKind.bankAccount,
    // Aquamarine.
    'wifi': Jewel(Glyphs.wifi, Color(0xFF1C7FB8), Color(0xFF1F86C2)),
    // Garnet.
    'server': Jewel(Glyphs.server, Color(0xFFA8291F), Color(0xFFB83328)),
    // Ruby.
    'license': Jewel(Glyphs.license, Color(0xFFB01E54), Color(0xFFC0275F)),
    'apiKey': Jewel(Glyphs.key, Color(0xFF353B4A), Color(0xFF4A5266)),
    'domain': Jewel(Glyphs.globe, Color(0xFF1C7FB8), Color(0xFF1F86C2)),
    'email': Jewel(Glyphs.envelope, Color(0xFF2448B8), Color(0xFF2E55C9)),
    'certificate': Jewel(Glyphs.badge, Color(0xFFB01E54), Color(0xFFC0275F)),
  };

  /// A category's jewel; a note's for one the app doesn't know.
  static Jewel of(String? category) => _looks[category] ?? ItemKind.note;
}

ThemeData passBundleTheme(Brightness brightness) {
  final dark = brightness == Brightness.dark;
  final colors = dark ? PassBundleColors.dark : PassBundleColors.light;
  final text = colors.text;
  final scheme = ColorScheme.fromSeed(
    seedColor: brandGreen,
    brightness: brightness,
    primary: colors.accent,
    onPrimary: colors.onAccent,
    error: colors.error,
    surface: colors.canvas,
    onSurface: text,
    onSurfaceVariant: colors.muted,
    outline: colors.outline,
    outlineVariant: colors.line,
    secondaryContainer: colors.selected,
    onSecondaryContainer: text,
    surfaceContainerLowest: colors.canvas,
    surfaceContainerLow: colors.canvas,
    surfaceContainer: colors.canvas,
    surfaceContainerHigh: colors.dialog,
    surfaceContainerHighest: colors.sunken,
    inverseSurface: colors.inverse,
    onInverseSurface: colors.onInverse,
    inversePrimary: dark ? brandGreen : null,
  );
  final base = ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    fontFamily: passBundleFontFamily,
    // In the dark each page stands on the ground and its washes (Ground,
    // put there by the page transitions), so pages themselves are clear.
    scaffoldBackgroundColor: dark ? Colors.transparent : colors.ground,
    pageTransitionsTheme: dark
        ? const _OnTheGround()
        : const PageTransitionsTheme(),
    splashFactory: NoSplash.splashFactory,
    extensions: [colors],
  );
  final shape = RoundedRectangleBorder(borderRadius: BorderRadius.circular(10));
  BorderSide edge(Color? color) =>
      color == null ? BorderSide.none : BorderSide(color: color);
  final floatingShape = RoundedRectangleBorder(
    borderRadius: BorderRadius.circular(20),
    side: edge(colors.dialogEdge),
  );
  return base.copyWith(
    textTheme: base.textTheme.apply(bodyColor: text, displayColor: text),
    appBarTheme: AppBarTheme(
      backgroundColor: dark ? Colors.transparent : colors.ground,
      surfaceTintColor: Colors.transparent,
      scrolledUnderElevation: 0,
      foregroundColor: text,
      systemOverlayStyle: colors.bars,
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: colors.field,
      isDense: true,
      contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: colors.fieldEdge),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: colors.fieldEdge),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: scheme.primary, width: 1.6),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        shape: shape,
        minimumSize: const Size(64, 48),
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 14),
        textStyle: const TextStyle(
          fontFamily: passBundleFontFamily,
          fontWeight: FontWeight.w600,
          fontSize: 15,
        ),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        shape: shape,
        minimumSize: const Size(64, 44),
        foregroundColor: text,
        backgroundColor: colors.button,
        side: BorderSide(color: colors.buttonEdge),
        textStyle: const TextStyle(
          fontFamily: passBundleFontFamily,
          fontWeight: FontWeight.w600,
          fontSize: 15,
        ),
      ),
    ),
    // In the dark, a choice's border is a quiet hairline, as a field's.
    segmentedButtonTheme: dark
        ? SegmentedButtonThemeData(
            style: ButtonStyle(
              side: WidgetStatePropertyAll(
                BorderSide(color: colors.buttonEdge),
              ),
            ),
          )
        : null,
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        shape: shape,
        minimumSize: const Size(48, 44),
        textStyle: const TextStyle(
          fontFamily: passBundleFontFamily,
          fontWeight: FontWeight.w600,
        ),
      ),
    ),
    iconButtonTheme: IconButtonThemeData(
      style: IconButton.styleFrom(
        shape: shape,
        minimumSize: const Size(44, 44),
      ),
    ),
    floatingActionButtonTheme: FloatingActionButtonThemeData(
      backgroundColor: scheme.primary,
      foregroundColor: scheme.onPrimary,
      elevation: 3,
      highlightElevation: 4,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(18)),
    ),
    navigationBarTheme: NavigationBarThemeData(
      backgroundColor: colors.pane,
      surfaceTintColor: Colors.transparent,
      indicatorColor: colors.selected,
      elevation: 0,
      height: 68,
      labelTextStyle: WidgetStateProperty.resolveWith(
        (states) => TextStyle(
          fontFamily: passBundleFontFamily,
          fontSize: 12,
          fontWeight: FontWeight.w600,
          color: states.contains(WidgetState.selected) ? text : colors.muted,
        ),
      ),
      iconTheme: WidgetStateProperty.resolveWith(
        (states) => IconThemeData(
          size: 22,
          color: states.contains(WidgetState.selected) ? text : colors.muted,
        ),
      ),
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: colors.dialog,
      surfaceTintColor: Colors.transparent,
      shape: floatingShape,
      // Room for words on a phone: the default leaves a narrow column.
      insetPadding: const EdgeInsets.symmetric(horizontal: 20, vertical: 24),
    ),
    bottomSheetTheme: BottomSheetThemeData(
      backgroundColor: colors.dialog,
      surfaceTintColor: Colors.transparent,
      modalBackgroundColor: colors.dialog,
      showDragHandle: true,
      shape: RoundedRectangleBorder(
        borderRadius: const BorderRadius.vertical(top: Radius.circular(24)),
        side: edge(colors.dialogEdge),
      ),
    ),
    popupMenuTheme: PopupMenuThemeData(
      color: colors.menu,
      surfaceTintColor: Colors.transparent,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: edge(colors.menuEdge),
      ),
    ),
    // The banner that says the server signed this device out: a strip of
    // the cards' surface, over the ground.
    bannerTheme: MaterialBannerThemeData(
      backgroundColor: colors.pane,
      dividerColor: colors.line,
    ),
    snackBarTheme: SnackBarThemeData(
      behavior: SnackBarBehavior.floating,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
    ),
    listTileTheme: ListTileThemeData(
      contentPadding: const EdgeInsets.symmetric(horizontal: 16),
      minTileHeight: 56,
      iconColor: colors.muted,
      subtitleTextStyle: TextStyle(
        fontFamily: passBundleFontFamily,
        fontSize: 14,
        color: colors.muted,
      ),
    ),
    switchTheme: SwitchThemeData(
      trackOutlineColor: WidgetStateProperty.resolveWith(
        (states) => states.contains(WidgetState.selected)
            ? Colors.transparent
            : scheme.outline,
      ),
    ),
    dividerTheme: DividerThemeData(
      color: colors.divider,
      thickness: 1,
      space: 1,
    ),
    // In the dark, the slider's track past the thumb is a hairline's white,
    // where the selection's tint wouldn't show on a card.
    sliderTheme: dark
        ? SliderThemeData(inactiveTrackColor: colors.buttonEdge)
        : null,
    tooltipTheme: const TooltipThemeData(
      waitDuration: Duration(milliseconds: 500),
    ),
  );
}

extension PassBundleTheme on BuildContext {
  PassBundleColors get colors {
    final theme = Theme.of(this);
    return theme.extension<PassBundleColors>() ??
        (theme.brightness == Brightness.dark
            ? PassBundleColors.dark
            : PassBundleColors.light);
  }
}

/// A section's heading in a list over the ground: Settings' groups, the
/// vault's Favorites and All items.
class SectionHeading extends StatelessWidget {
  const SectionHeading(
    this.text, {
    super.key,
    this.padding = const EdgeInsets.fromLTRB(16, 24, 16, 8),
    this.trailing,
    this.height = 0,
  });

  final String text;
  final EdgeInsetsGeometry padding;
  final Widget? trailing;

  /// At least this tall: headings with a button beside them and without keep
  /// the same spacing when they're all this tall.
  final double height;

  @override
  Widget build(BuildContext context) => Padding(
    padding: padding,
    child: ConstrainedBox(
      constraints: BoxConstraints(minHeight: height),
      child: Row(
        children: [
          Expanded(
            child: Semantics(
              header: true,
              child: Text(
                text,
                style: TextStyle(
                  color: context.colors.heading,
                  fontSize: 13,
                  fontWeight: FontWeight.w700,
                  letterSpacing: 0.2,
                ),
              ),
            ),
          ),
          ?trailing,
        ],
      ),
    ),
  );
}

/// A card resting on the ground: white, or in the dark translucent glass.
class RestingCard extends StatelessWidget {
  const RestingCard({super.key, required this.child, this.radius = 16});

  final Widget child;
  final double radius;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return DecoratedBox(
      decoration: c.resting.decoration(BorderRadius.circular(radius)),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(radius),
        child: Material(type: MaterialType.transparency, child: child),
      ),
    );
  }
}

/// A group of settings: a heading, its rows on a resting card, and a line
/// under them. Rows are list tiles with 16 on each side, so their titles,
/// subtitles, and switches line up from group to group.
class SettingsGroup extends StatelessWidget {
  const SettingsGroup({
    super.key,
    required this.title,
    required this.children,
    this.footer,
    this.footerColor,
  });

  final String title;
  final List<Widget> children;

  /// Said under the card: what an organization's policy does, say.
  final String? footer;
  final Color? footerColor;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    mainAxisSize: MainAxisSize.min,
    children: [
      SectionHeading(title, padding: const EdgeInsets.fromLTRB(4, 20, 4, 8)),
      RestingCard(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: MainAxisSize.min,
          children: [
            for (var i = 0; i < children.length; i++) ...[
              if (i > 0) const Divider(indent: 16),
              children[i],
            ],
          ],
        ),
      ),
      if (footer != null)
        Padding(
          padding: const EdgeInsets.fromLTRB(4, 8, 4, 0),
          // A node of its own, read where it is, under its group.
          child: Semantics(
            container: true,
            child: Text(
              footer!,
              style: TextStyle(
                color: footerColor ?? context.colors.muted,
                fontSize: 13,
                height: 1.35,
              ),
            ),
          ),
        ),
    ],
  );
}
