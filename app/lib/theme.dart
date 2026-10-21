// PassBundle's own look, the same on every platform: brand colors, Inter
// (bundled, so no system font changes it), and our own shapes. Light and dark,
// or following the system.
import 'package:flutter/material.dart';

const brandGreen = Color(0xFF1F6F5C);

class PassBundleColors extends ThemeExtension<PassBundleColors> {
  const PassBundleColors({
    required this.sidebar,
    required this.sidebarText,
    required this.pane,
    required this.line,
    required this.muted,
    required this.selected,
  });

  final Color sidebar;
  final Color sidebarText;
  final Color pane;
  final Color line;
  final Color muted;
  final Color selected;

  static const light = PassBundleColors(
    sidebar: Color(0xFF12453A),
    sidebarText: Color(0xFFE6F2EE),
    pane: Color(0xFFFFFFFF),
    line: Color(0xFFE3E8E6),
    muted: Color(0xFF5E6B67),
    selected: Color(0xFFE3F1EC),
  );

  static const dark = PassBundleColors(
    sidebar: Color(0xFF0B1714),
    sidebarText: Color(0xFFCFE3DC),
    pane: Color(0xFF141C1A),
    line: Color(0xFF26312E),
    muted: Color(0xFF93A39E),
    selected: Color(0xFF1C3A32),
  );

  @override
  PassBundleColors copyWith() => this;

  @override
  PassBundleColors lerp(ThemeExtension<PassBundleColors>? other, double t) =>
      t < 0.5 || other is! PassBundleColors ? this : other;
}

ThemeData passBundleTheme(Brightness brightness) {
  final dark = brightness == Brightness.dark;
  final scheme = ColorScheme.fromSeed(
    seedColor: brandGreen,
    brightness: brightness,
    primary: dark ? const Color(0xFF6FCFB2) : brandGreen,
    surface: dark ? const Color(0xFF101614) : const Color(0xFFF6F8F7),
  );
  final colors = dark ? PassBundleColors.dark : PassBundleColors.light;
  final base = ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    fontFamily: 'Inter',
    scaffoldBackgroundColor: scheme.surface,
    splashFactory: NoSplash.splashFactory,
    extensions: [colors],
  );
  final shape = RoundedRectangleBorder(borderRadius: BorderRadius.circular(10));
  return base.copyWith(
    textTheme: base.textTheme.apply(
      bodyColor: dark ? const Color(0xFFE4ECE9) : const Color(0xFF17201D),
      displayColor: dark ? const Color(0xFFE4ECE9) : const Color(0xFF17201D),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: colors.pane,
      isDense: true,
      contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: BorderSide(color: colors.line),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: BorderSide(color: colors.line),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: BorderSide(color: scheme.primary, width: 1.6),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        shape: shape,
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 14),
        textStyle: const TextStyle(
          fontFamily: 'Inter',
          fontWeight: FontWeight.w600,
        ),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(shape: shape),
    ),
    iconButtonTheme: IconButtonThemeData(
      style: IconButton.styleFrom(shape: shape),
    ),
    dividerTheme: DividerThemeData(color: colors.line, thickness: 1, space: 1),
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
