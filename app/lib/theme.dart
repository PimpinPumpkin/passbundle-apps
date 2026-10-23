// PassBundle's own look, the same on every platform: brand colors, Google
// Sans Flex (bundled, so no system font changes it), and our own shapes. Light
// and dark, or following the system. The design language: a calm, solid
// ground; white (or, in the dark, slightly lighter) surfaces that rise in three
// steps (resting, raised, floating); and a jewel-toned color for each kind of
// item.
import 'package:flutter/material.dart';

import 'glyphs.dart';

const brandGreen = Color(0xFF1F6F5C);

/// The typeface, everywhere: one variable font, whose weight axis follows
/// each style's FontWeight (pubspec.yaml).
const passBundleFontFamily = 'Google Sans Flex';

class PassBundleColors extends ThemeExtension<PassBundleColors> {
  const PassBundleColors({
    required this.pane,
    required this.line,
    required this.muted,
    required this.selected,
    required this.ground,
    required this.raised,
    required this.divider,
    required this.tint,
    required this.well,
    required this.heading,
    required this.resting,
    required this.lifted,
    required this.floating,
  });

  /// A surface on the ground: lists, cards, panels.
  final Color pane;
  final Color line;
  final Color muted;
  final Color selected;

  /// The calm, solid ground everything sits on.
  final Color ground;

  /// A surface a step up from [pane]: a selected item, a sheet.
  final Color raised;

  /// Between the rows of a card.
  final Color divider;

  /// The quiet fill behind a copy button, in the brand color's family.
  final Color tint;

  /// Set into a raised card: an item's details on a computer.
  final Color well;

  /// A section's heading, over the ground.
  final Color heading;

  /// The three levels of depth: lists and panels; cards and the selected
  /// item; menus, sheets, and dialogs.
  final List<BoxShadow> resting;
  final List<BoxShadow> lifted;
  final List<BoxShadow> floating;

  static const light = PassBundleColors(
    pane: Color(0xFFFFFFFF),
    line: Color(0xFFDCE5E1),
    muted: Color(0xFF52615C),
    selected: Color(0xFFD6EBE3),
    ground: Color(0xFFEEF2F0),
    raised: Color(0xFFFFFFFF),
    divider: Color(0xFFEDF2F0),
    tint: Color(0xFFE7F1ED),
    well: Color(0xFFF7FAF9),
    heading: Color(0xFF3C4A45),
    resting: [
      BoxShadow(color: Color(0x0F10201B), offset: Offset(0, 1), blurRadius: 2),
      BoxShadow(color: Color(0x0A10201B), offset: Offset(0, 1), blurRadius: 1),
    ],
    lifted: [
      BoxShadow(color: Color(0x1410201B), offset: Offset(0, 4), blurRadius: 12),
      BoxShadow(color: Color(0x0F10201B), offset: Offset(0, 1), blurRadius: 3),
    ],
    floating: [
      BoxShadow(
        color: Color(0x2410201B),
        offset: Offset(0, 16),
        blurRadius: 40,
      ),
      BoxShadow(color: Color(0x1410201B), offset: Offset(0, 4), blurRadius: 12),
    ],
  );

  static const dark = PassBundleColors(
    pane: Color(0xFF16201D),
    line: Color(0xFF2A3833),
    muted: Color(0xFFA3B2AD),
    selected: Color(0xFF1F3A32),
    ground: Color(0xFF0E1513),
    raised: Color(0xFF1C2824),
    divider: Color(0xFF22302B),
    tint: Color(0xFF1F3A32),
    well: Color(0xFF1C2824),
    heading: Color(0xFFB9C6C2),
    // In the dark a shadow barely shows: an edge does the work.
    resting: [BoxShadow(color: Color(0xFF22302B), spreadRadius: 1)],
    lifted: [
      BoxShadow(color: Color(0xFF2A3833), spreadRadius: 1),
      BoxShadow(color: Color(0x59000000), offset: Offset(0, 6), blurRadius: 16),
    ],
    floating: [
      BoxShadow(color: Color(0xFF34443F), spreadRadius: 1),
      BoxShadow(
        color: Color(0x66000000),
        offset: Offset(0, 16),
        blurRadius: 40,
      ),
    ],
  );

  @override
  PassBundleColors copyWith() => this;

  @override
  PassBundleColors lerp(ThemeExtension<PassBundleColors>? other, double t) =>
      t < 0.5 || other is! PassBundleColors ? this : other;
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

  /// In the order the vault lists them.
  static const all = [login, card, identity, note, sshKey];

  static ItemKind of(String kind) =>
      all.where((k) => k.kind == kind).firstOrNull ?? login;
}

ThemeData passBundleTheme(Brightness brightness) {
  final dark = brightness == Brightness.dark;
  final colors = dark ? PassBundleColors.dark : PassBundleColors.light;
  final text = dark ? const Color(0xFFE7EFEC) : const Color(0xFF14201C);
  final scheme = ColorScheme.fromSeed(
    seedColor: brandGreen,
    brightness: brightness,
    primary: dark ? const Color(0xFF5FC3A6) : brandGreen,
    onPrimary: dark ? const Color(0xFF0B1F19) : Colors.white,
    surface: colors.pane,
    onSurface: text,
    onSurfaceVariant: colors.muted,
    outline: dark ? const Color(0xFF34443F) : const Color(0xFFCBD7D2),
    outlineVariant: colors.line,
    secondaryContainer: colors.selected,
    onSecondaryContainer: text,
    surfaceContainerLowest: colors.pane,
    surfaceContainerLow: colors.pane,
    surfaceContainer: colors.pane,
    surfaceContainerHigh: colors.raised,
    surfaceContainerHighest: dark
        ? const Color(0xFF22302B)
        : const Color(0xFFE7EEEB),
  );
  final base = ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    fontFamily: passBundleFontFamily,
    scaffoldBackgroundColor: colors.ground,
    splashFactory: NoSplash.splashFactory,
    extensions: [colors],
  );
  final shape = RoundedRectangleBorder(borderRadius: BorderRadius.circular(10));
  final floatingShape = RoundedRectangleBorder(
    borderRadius: BorderRadius.circular(20),
    side: dark ? BorderSide(color: colors.line) : BorderSide.none,
  );
  return base.copyWith(
    textTheme: base.textTheme.apply(bodyColor: text, displayColor: text),
    appBarTheme: AppBarTheme(
      backgroundColor: colors.ground,
      surfaceTintColor: Colors.transparent,
      scrolledUnderElevation: 0,
      foregroundColor: text,
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: colors.pane,
      isDense: true,
      contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: colors.line),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: colors.line),
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
        backgroundColor: colors.pane,
        side: BorderSide(color: scheme.outline),
        textStyle: const TextStyle(
          fontFamily: passBundleFontFamily,
          fontWeight: FontWeight.w600,
          fontSize: 15,
        ),
      ),
    ),
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
      backgroundColor: colors.raised,
      surfaceTintColor: Colors.transparent,
      shape: floatingShape,
      // Room for words on a phone: the default leaves a narrow column.
      insetPadding: const EdgeInsets.symmetric(horizontal: 20, vertical: 24),
    ),
    bottomSheetTheme: BottomSheetThemeData(
      backgroundColor: colors.raised,
      surfaceTintColor: Colors.transparent,
      modalBackgroundColor: colors.raised,
      showDragHandle: true,
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(24)),
      ),
    ),
    popupMenuTheme: PopupMenuThemeData(
      color: colors.raised,
      surfaceTintColor: Colors.transparent,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: dark ? BorderSide(color: colors.line) : BorderSide.none,
      ),
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

/// A white (or, in the dark, slightly lighter) card resting on the ground.
class RestingCard extends StatelessWidget {
  const RestingCard({super.key, required this.child, this.radius = 16});

  final Widget child;
  final double radius;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return DecoratedBox(
      decoration: BoxDecoration(
        color: c.pane,
        borderRadius: BorderRadius.circular(radius),
        boxShadow: c.resting,
      ),
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
