// Small pieces every screen uses: the mark, item avatars, the busy flag, and
// the theme menu.
import 'package:flutter/material.dart';

import 'core.dart';
import 'glyphs.dart';
import 'theme.dart';

/// True while a slow core call runs; the screens don't read from the core then.
final busy = ValueNotifier<bool>(false);

/// Runs a slow call with the busy flag up.
Future<Answer> whileBusy(Future<Answer> Function() call) async {
  busy.value = true;
  try {
    return await call();
  } finally {
    busy.value = false;
  }
}

/// The PassBundle mark: a padlock whose body is three bands.
class Mark extends StatelessWidget {
  const Mark({super.key, this.size = 28, this.color});

  final double size;
  final Color? color;

  @override
  Widget build(BuildContext context) => CustomPaint(
    size: Size(size * 26 / 32, size),
    painter: _MarkPainter(color ?? Theme.of(context).colorScheme.primary),
  );
}

class _MarkPainter extends CustomPainter {
  _MarkPainter(this.color);

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / 26, size.height / 32);
    const r85 = Radius.circular(8.5), r45 = Radius.circular(4.5);
    final path = Path()
      // The shackle.
      ..moveTo(4.5, 13)
      ..lineTo(4.5, 8.5)
      ..arcToPoint(const Offset(21.5, 8.5), radius: r85)
      ..lineTo(21.5, 13)
      ..lineTo(17.5, 13)
      ..lineTo(17.5, 8.5)
      ..arcToPoint(const Offset(8.5, 8.5), radius: r45, clockwise: false)
      ..lineTo(8.5, 13)
      ..close()
      // The top band.
      ..moveTo(4.5, 13)
      ..lineTo(21.5, 13)
      ..arcToPoint(const Offset(26, 17.5), radius: r45)
      ..lineTo(26, 18)
      ..lineTo(0, 18)
      ..lineTo(0, 17.5)
      ..arcToPoint(const Offset(4.5, 13), radius: r45)
      ..close()
      // The middle band.
      ..addRect(const Rect.fromLTWH(0, 20, 26, 5))
      // The bottom band.
      ..moveTo(0, 27)
      ..lineTo(26, 27)
      ..lineTo(26, 27.5)
      ..arcToPoint(const Offset(21.5, 32), radius: r45)
      ..lineTo(4.5, 32)
      ..arcToPoint(const Offset(0, 27.5), radius: r45)
      ..close();
    canvas.drawPath(path, Paint()..color = color);
  }

  @override
  bool shouldRepaint(_MarkPainter old) => old.color != color;
}

/// PassBundle's logo, as the web vault and the extension show it
/// (passbundle-clients, brand/wordmark.py): the emerald jewel tile with the
/// mark in white, then the name in lower case, "pass" bold and "bundle"
/// lighter, in the brand's two tones. [size] is the tile's; the rest keeps
/// the logo's proportions (a 42-unit tile, 12 to the name, the name at 31).
class Wordmark extends StatelessWidget {
  const Wordmark({super.key, this.size = 42});

  final double size;

  static const _emerald = Color(0xFF0E7A5A);

  @override
  Widget build(BuildContext context) {
    final dark = Theme.of(context).brightness == Brightness.dark;
    final u = size / 42;
    final font = 31 * u;
    TextStyle style(double weight, Color color) => TextStyle(
      fontFamily: passBundleFontFamily,
      fontSize: font,
      fontWeight: weight >= 600 ? FontWeight.w700 : FontWeight.w400,
      fontVariations: [FontVariation('wght', weight)],
      letterSpacing: -0.02 * font,
      height: 1,
      color: color,
    );
    return Semantics(
      label: 'PassBundle',
      excludeSemantics: true,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          JewelTile(
            jewel: Jewel.allItems,
            size: size,
            // The lock is 18 of the tile's 42 units wide.
            child: Mark(size: 18 * u * 32 / 26, color: Colors.white),
          ),
          SizedBox(width: 12 * u),
          // The logo doesn't grow with the text size: it's a picture.
          Text.rich(
            TextSpan(
              children: [
                TextSpan(
                  text: 'pass',
                  style: style(
                    700,
                    dark ? const Color(0xFFE7EFEC) : const Color(0xFF1B3A33),
                  ),
                ),
                TextSpan(
                  text: 'bundle',
                  style: style(
                    450,
                    dark ? const Color(0xFF6FD4B4) : _emerald,
                  ),
                ),
              ],
            ),
            textScaler: TextScaler.noScaling,
            maxLines: 1,
            softWrap: false,
          ),
        ],
      ),
    );
  }
}

/// An item's tile: a login's first letter on a tint of the login color (a
/// site's own icon is for later), and for the other kinds their kind's jewel
/// tile, so a list is never a column of identical keys. The same on a phone,
/// a computer, and in the web vault.
class Avatar extends StatelessWidget {
  const Avatar({
    super.key,
    required this.name,
    required this.kind,
    this.size = 36,
  });

  final String name;
  final String kind;
  final double size;

  /// The first letter or digit of a name, or nothing when it has none.
  static String letter(String name) {
    for (final c in name.characters) {
      if (RegExp(r'[\p{L}\p{N}]', unicode: true).hasMatch(c)) {
        return c.toUpperCase();
      }
    }
    return '';
  }

  @override
  Widget build(BuildContext context) {
    if (kind != 'login') {
      return JewelTile(jewel: ItemKind.of(kind), size: size);
    }
    final dark = Theme.of(context).brightness == Brightness.dark;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: dark ? const Color(0xFF1C2A4A) : const Color(0xFFE3E9F8),
        borderRadius: BorderRadius.circular(size * 0.28),
      ),
      alignment: Alignment.center,
      child: Text(
        letter(name),
        // The tile doesn't grow with the text size, so its letter doesn't either.
        textScaler: TextScaler.noScaling,
        style: TextStyle(
          color: dark ? const Color(0xFF9DB4F2) : const Color(0xFF1D3C99),
          fontWeight: FontWeight.w700,
          fontSize: size * 0.44,
        ),
      ),
    );
  }
}

/// A jewel tile: the jewel's tone with its white glyph, a little sheen from
/// the top left, and a shade at the bottom, like a cut stone. The same in the
/// menu, the list, an item, and the New picker, on every screen.
class JewelTile extends StatelessWidget {
  const JewelTile({super.key, required this.jewel, this.size = 36, this.child});

  final Jewel jewel;
  final double size;

  /// In place of the jewel's glyph: the mark, on the menu's emerald.
  final Widget? child;

  @override
  Widget build(BuildContext context) {
    final brightness = Theme.of(context).brightness;
    final dark = brightness == Brightness.dark;
    final radius = BorderRadius.circular(size * 0.28);
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: jewel.color(brightness),
        borderRadius: radius,
        boxShadow: [
          BoxShadow(
            color: dark ? const Color(0x40000000) : const Color(0x3810201B),
            offset: const Offset(0, 1),
            blurRadius: 2,
          ),
        ],
      ),
      child: ClipRRect(
        borderRadius: radius,
        child: Stack(
          fit: StackFit.expand,
          children: [
            // The sheen, from the top left: a two-color gradient, which
            // every renderer draws without a texture of its own.
            DecoratedBox(
              decoration: BoxDecoration(
                gradient: RadialGradient(
                  center: const Alignment(-0.44, -0.64),
                  radius: 0.55,
                  colors: [
                    Color(dark ? 0x4DFFFFFF : 0x52FFFFFF),
                    const Color(0x00FFFFFF),
                  ],
                ),
              ),
            ),
            // The light along the top edge, and the shade at the bottom.
            const Positioned(
              top: 0,
              left: 0,
              right: 0,
              height: 1,
              child: ColoredBox(color: Color(0x59FFFFFF)),
            ),
            Positioned(
              left: 0,
              right: 0,
              bottom: 0,
              height: size * 0.22,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    begin: Alignment.topCenter,
                    end: Alignment.bottomCenter,
                    colors: [
                      const Color(0x00000000),
                      Color(dark ? 0x40000000 : 0x33000000),
                    ],
                  ),
                ),
              ),
            ),
            Center(
              child:
                  child ??
                  Glyph(
                    jewel.glyph,
                    size: size * 0.56,
                    color: Colors.white,
                    strokeWidth: size < 28 ? 2.2 : 1.9,
                  ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Light, dark, or following the system.
class ThemeMenu extends StatelessWidget {
  const ThemeMenu({
    super.key,
    required this.mode,
    required this.onMode,
    this.color,
  });

  final ThemeMode mode;
  final ValueChanged<ThemeMode> onMode;
  final Color? color;

  @override
  Widget build(BuildContext context) => PopupMenuButton<ThemeMode>(
    tooltip: 'Theme',
    icon: Icon(Icons.contrast_rounded, color: color),
    initialValue: mode,
    onSelected: onMode,
    itemBuilder: (_) => const [
      PopupMenuItem(value: ThemeMode.system, child: Text('Follow the system')),
      PopupMenuItem(value: ThemeMode.light, child: Text('Light')),
      PopupMenuItem(value: ThemeMode.dark, child: Text('Dark')),
    ],
  );
}

/// A centered card for the sign-in and unlock screens.
class Centered extends StatelessWidget {
  const Centered({super.key, required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Scaffold(
    body: Center(
      child: SingleChildScrollView(
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 24),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 400),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: children,
          ),
        ),
      ),
    ),
  );
}

Widget heading(BuildContext context, String title, String? subtitle) => Column(
  children: [
    const Center(child: Wordmark(size: 44)),
    const SizedBox(height: 28),
    Text(
      title,
      textAlign: TextAlign.center,
      style: Theme.of(context).textTheme.headlineSmall
          ?.copyWith(fontWeight: FontWeight.w700),
    ),
    if (subtitle != null) ...[
      const SizedBox(height: 6),
      Text(
        subtitle,
        textAlign: TextAlign.center,
        style: TextStyle(color: context.colors.muted),
      ),
    ],
    const SizedBox(height: 28),
  ],
);

/// A button that shows a spinner while its work runs.
class WorkingButton extends StatelessWidget {
  const WorkingButton({
    super.key,
    required this.label,
    required this.working,
    required this.onPressed,
  });

  final String label;
  final bool working;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => FilledButton(
    onPressed: working ? null : onPressed,
    child: working
        ? const SizedBox(
            width: 18,
            height: 18,
            child: CircularProgressIndicator(strokeWidth: 2),
          )
        : Text(label),
  );
}

/// One of a few choices, in a dialog: null if dismissed. A list of radio
/// rows, so each choice has the width it needs, at any text size.
Future<T?> chooseOne<T>(
  BuildContext context, {
  required String title,
  String? description,
  required T selected,
  required List<(T, String)> choices,
}) => showDialog<T>(
  context: context,
  builder: (context) => AlertDialog(
    title: Text(title),
    contentPadding: const EdgeInsets.fromLTRB(0, 16, 0, 8),
    content: SingleChildScrollView(
      child: RadioGroup<T>(
        groupValue: selected,
        onChanged: (value) => Navigator.pop(context, value),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (description != null)
              Padding(
                padding: const EdgeInsets.fromLTRB(24, 0, 24, 8),
                child: Text(
                  description,
                  style: TextStyle(color: context.colors.muted),
                ),
              ),
            for (final (value, label) in choices)
              RadioListTile<T>(
                value: value,
                title: Text(label),
                contentPadding: const EdgeInsets.symmetric(horizontal: 12),
              ),
          ],
        ),
      ),
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.pop(context),
        child: const Text('Cancel'),
      ),
    ],
  ),
);

/// A phone screen's title, big, with what goes beside it: the vault's Sync
/// and Lock, say, or a way back.
class PageTitle extends StatelessWidget {
  const PageTitle(
    this.title, {
    super.key,
    this.leading,
    this.actions = const [],
  });

  final String title;
  final Widget? leading;
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) => Padding(
    padding: EdgeInsets.fromLTRB(leading == null ? 20 : 6, 8, 8, 4),
    child: ConstrainedBox(
      constraints: const BoxConstraints(minHeight: 52),
      child: Row(
        children: [
          ?leading,
          Expanded(
            child: Semantics(
              header: true,
              child: Text(
                title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontSize: leading == null ? 30 : 24,
                  fontWeight: FontWeight.w700,
                  letterSpacing: -0.3,
                ),
              ),
            ),
          ),
          ...actions,
        ],
      ),
    ),
  );
}
