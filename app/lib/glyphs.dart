// The glyphs the web vault and the browser extension draw (passbundle-clients,
// libs/common/src/vault/keykeeper/categories.ts, and the design boards): SVG
// path data on a 24 by 24 grid, stroked with round ends, so the apps show the
// same icons. A small reader for SVG's path commands turns them into Flutter
// paths, once each.
import 'package:flutter/material.dart';

/// The glyphs, by what they show.
abstract final class Glyphs {
  // A kind of item each (categories.ts).
  static const grid = 'M4 4h7v7H4Z M13 4h7v7h-7Z M4 13h7v7H4Z M13 13h7v7h-7Z';
  static const key =
      'M8 14a4 4 0 1 1 3.87-5H21v3h-2v3h-3v-3h-4.13A4 4 0 0 1 8 14Z M7 10h.01';
  static const note = 'M6 3h9l4 4v14H6Z M14 3v5h5 M9 12h7 M9 16h5';
  static const card = 'M3 6h18v12H3Z M3 10h18 M7 15h4';
  static const identity =
      'M4 5h16v14H4Z M9 11a2 2 0 1 0 0-4 2 2 0 0 0 0 4Z M5.5 16c.6-1.8 2-2.8 3.5-2.8s2.9 1 3.5 2.8 M14 9h4 M14 13h4';
  static const terminal = 'M3 5h18v14H3Z M7 10l3 2-3 2 M12 15h5';

  // The web vault's menu and the phone's tabs (WebVault, PhoneVault).
  static const star =
      'M12 4l2.5 5 5.5.8-4 3.9.9 5.5L12 16.6 7.1 19.2l.9-5.5-4-3.9 5.5-.8Z';
  static const folder = 'M3 6h6l2 2h10v11H3Z';
  static const building =
      'M4 21V5h10v16 M14 9h6v12 M7 8h3 M7 12h3 M7 16h3 M3 21h18';
  static const settings =
      'M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.4-2.3.9a7 7 0 0 0-2.1-1.2L14 3h-4l-.5 2.6a7 7 0 0 0-2.1 1.2l-2.3-.9-2 3.4 2 1.5A7 7 0 0 0 5 12c0 .4 0 .8.1 1.2l-2 1.5 2 3.4 2.3-.9a7 7 0 0 0 2.1 1.2L10 21h4l.5-2.6a7 7 0 0 0 2.1-1.2l2.3.9 2-3.4-2-1.5c.1-.4.1-.8.1-1.2Z';

  // What's done with an item (PhoneItem, PhoneVault).
  static const search = 'M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14Z M20 20l-4-4';
  static const copy = 'M9 9h11v11H9Z M5 15H4V4h11v1';
  static const eye =
      'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z';
  static const eyeOff =
      'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z M4 4l16 16';
  static const chevron = 'M9 6l6 6-6 6';
  static const plus = 'M12 5v14 M5 12h14';
}

/// A glyph, stroked (or filled) in the icon color.
class Glyph extends StatelessWidget {
  const Glyph(
    this.data, {
    super.key,
    this.size = 24,
    this.color,
    this.strokeWidth = 1.9,
    this.fill,
    this.semanticLabel,
  });

  /// SVG path data on a 24 by 24 grid.
  final String data;
  final double size;
  final Color? color;

  /// On the 24 grid: it scales with the glyph.
  final double strokeWidth;

  /// Filled with this color as well (a favorite's star).
  final Color? fill;
  final String? semanticLabel;

  @override
  Widget build(BuildContext context) {
    final icon = IconTheme.of(context);
    final paint = CustomPaint(
      size: Size.square(size),
      painter: _GlyphPainter(
        svgPath(data),
        color ?? icon.color ?? Theme.of(context).colorScheme.onSurface,
        strokeWidth,
        fill,
      ),
    );
    return semanticLabel == null
        ? ExcludeSemantics(child: paint)
        : Semantics(label: semanticLabel, child: paint);
  }
}

class _GlyphPainter extends CustomPainter {
  _GlyphPainter(this.path, this.color, this.strokeWidth, this.fill);

  final Path path;
  final Color color;
  final double strokeWidth;
  final Color? fill;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / 24, size.height / 24);
    if (fill != null) {
      canvas.drawPath(path, Paint()..color = fill!);
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = color
        ..style = PaintingStyle.stroke
        ..strokeWidth = strokeWidth
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round,
    );
  }

  @override
  bool shouldRepaint(_GlyphPainter old) =>
      old.path != path ||
      old.color != color ||
      old.strokeWidth != strokeWidth ||
      old.fill != fill;
}

final _paths = <String, Path>{};

/// SVG path data as a Flutter path: moves, lines (and horizontal and vertical
/// ones), cubic and quadratic curves (and their smooth forms), arcs, and
/// closing, absolute and relative.
Path svgPath(String data) => _paths.putIfAbsent(data, () => _read(data));

Path _read(String data) {
  final path = Path();
  final tokens = RegExp(
    r'[MmLlHhVvCcSsQqTtAaZz]|[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?',
  ).allMatches(data).map((m) => m.group(0)!).toList();
  var i = 0;
  var command = '';
  var x = 0.0, y = 0.0;
  var startX = 0.0, startY = 0.0;
  // The last curve's second control point, for the smooth forms.
  double? cx, cy;
  String previous = '';
  bool isCommand(String t) => RegExp(r'^[A-Za-z]$').hasMatch(t);
  double next() => double.parse(tokens[i++]);
  // Arc flags may be written together ("a2 2 0 1 1 ..." or "a2 2 0 11 ...").
  bool flag() {
    final t = tokens[i];
    if (t.length > 1 && (t[0] == '0' || t[0] == '1')) {
      tokens[i] = t.substring(1);
      return t[0] == '1';
    }
    i++;
    return t == '1';
  }

  while (i < tokens.length) {
    if (isCommand(tokens[i])) {
      command = tokens[i++];
    } else if (command == 'M') {
      command = 'L';
    } else if (command == 'm') {
      command = 'l';
    }
    final relative = command == command.toLowerCase();
    final ox = relative ? x : 0.0, oy = relative ? y : 0.0;
    switch (command.toUpperCase()) {
      case 'M':
        x = ox + next();
        y = oy + next();
        path.moveTo(x, y);
        startX = x;
        startY = y;
        cx = cy = null;
      case 'L':
        x = ox + next();
        y = oy + next();
        path.lineTo(x, y);
        cx = cy = null;
      case 'H':
        x = ox + next();
        path.lineTo(x, y);
        cx = cy = null;
      case 'V':
        y = oy + next();
        path.lineTo(x, y);
        cx = cy = null;
      case 'C':
        final x1 = ox + next(), y1 = oy + next();
        final x2 = ox + next(), y2 = oy + next();
        x = ox + next();
        y = oy + next();
        path.cubicTo(x1, y1, x2, y2, x, y);
        cx = x2;
        cy = y2;
      case 'S':
        final smooth = 'CcSs'.contains(previous) && cx != null;
        final x1 = smooth ? 2 * x - cx : x, y1 = smooth ? 2 * y - cy! : y;
        final x2 = ox + next(), y2 = oy + next();
        x = ox + next();
        y = oy + next();
        path.cubicTo(x1, y1, x2, y2, x, y);
        cx = x2;
        cy = y2;
      case 'Q':
        final x1 = ox + next(), y1 = oy + next();
        x = ox + next();
        y = oy + next();
        path.quadraticBezierTo(x1, y1, x, y);
        cx = x1;
        cy = y1;
      case 'T':
        final smooth = 'QqTt'.contains(previous) && cx != null;
        final x1 = smooth ? 2 * x - cx : x, y1 = smooth ? 2 * y - cy! : y;
        x = ox + next();
        y = oy + next();
        path.quadraticBezierTo(x1, y1, x, y);
        cx = x1;
        cy = y1;
      case 'A':
        final rx = next().abs(), ry = next().abs();
        // In degrees, as arcToPoint takes it.
        final rotation = next();
        final large = flag(), sweep = flag();
        x = ox + next();
        y = oy + next();
        path.arcToPoint(
          Offset(x, y),
          radius: Radius.elliptical(rx, ry),
          rotation: rotation,
          largeArc: large,
          clockwise: sweep,
        );
        cx = cy = null;
      case 'Z':
        path.close();
        x = startX;
        y = startY;
        cx = cy = null;
      default:
        // Not path data: stop rather than loop.
        return path;
    }
    previous = command;
  }
  return path;
}
