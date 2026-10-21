// Small pieces every screen uses: the mark, item avatars, the busy flag, and
// the theme menu.
import 'package:flutter/material.dart';

import 'core.dart';
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

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: scheme.primaryContainer,
        borderRadius: BorderRadius.circular(size * 0.28),
      ),
      alignment: Alignment.center,
      child: kind == 'note'
          ? Icon(
              Icons.sticky_note_2_outlined,
              size: size * 0.5,
              color: scheme.onPrimaryContainer,
            )
          : Text(
              name.isEmpty ? '?' : name.characters.first.toUpperCase(),
              style: TextStyle(
                color: scheme.onPrimaryContainer,
                fontWeight: FontWeight.w700,
                fontSize: size * 0.42,
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
        padding: const EdgeInsets.all(24),
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
    const Center(child: Mark(size: 48)),
    const SizedBox(height: 20),
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
