// Websites' icons on logins' tiles, as the web vault and the extension show
// them: from the server's icon service, through the core, which keeps them in
// memory while the vault is open (core/src/icons.rs). A few at a time, once
// each, drawn no larger than a tile needs, and none at all when Settings
// turns them off. Nothing of them is written to the device.
import 'dart:async';
import 'dart:collection';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';

import 'core.dart';

/// Whether logins show their websites' icons (Settings, "Show website
/// icons"): read once from the core, and told when it changes.
final siteIcons = SiteIcons();

class SiteIcons {
  /// The most a side of an icon may be, as the core has it (`MAX_SIDE`):
  /// one that says it's larger isn't drawn at all.
  static const maxSide = 256;

  /// How large icons are drawn, at most, in pixels: a tile's size on the
  /// densest screens.
  static const drawn = 128;

  /// Each host's icon, drawn, or null for none, once known.
  final _known = <String, ui.Image?>{};
  final _waiting = <String, Completer<ui.Image?>>{};
  final _queue = Queue<String>();
  int _running = 0;

  /// Bumped by [forget], so an answer asked for before isn't kept after.
  int _session = 0;

  /// Bumped when icons are turned on or off, so tiles ask again.
  final changes = StreamController<void>.broadcast();

  bool? _on;
  bool get on => _on ??= Core().appearance()['showIcons'] != false;

  void turn(bool on) {
    _on = on;
    _forgetIcons();
    changes.add(null);
  }

  /// Forgets every icon, and the setting, as the vault closes (it locks, or
  /// the person signs out): the next vault's are asked for again.
  void forget() {
    _on = null;
    _forgetIcons();
  }

  void _forgetIcons() {
    _session++;
    _known.clear();
    _queue.clear();
    for (final waiting in _waiting.values) {
      waiting.complete(null);
    }
    _waiting.clear();
  }

  /// What's known for a host now, without waiting: the icon, null for none,
  /// or [unknown].
  Object? now(String host) => _known.containsKey(host) ? _known[host] : unknown;

  static const unknown = Object();

  /// A host's icon, or null: asked of the core four at a time.
  Future<ui.Image?> of(String host) {
    if (!on) return Future.value(null);
    if (_known.containsKey(host)) return Future.value(_known[host]);
    final waiting = _waiting[host];
    if (waiting != null) return waiting.future;
    final made = Completer<ui.Image?>();
    _waiting[host] = made;
    _queue.add(host);
    _next();
    return made.future;
  }

  void _next() {
    while (_running < 4 && _queue.isNotEmpty) {
      final host = _queue.removeFirst();
      final session = _session;
      _running++;
      Core.icon(host)
          .then<ui.Image?>(
            (bytes) => bytes == null ? null : draw(bytes),
            onError: (Object _) => null,
          )
          .then((icon) {
            _running--;
            if (session == _session) {
              _known[host] = icon;
              _waiting.remove(host)?.complete(icon);
            }
            _next();
          });
    }
  }

  /// An icon drawn from its file, no larger than [drawn] a side; null for
  /// a file whose header says it's larger than an icon ([maxSide]), which
  /// isn't drawn at all, or one Flutter doesn't read (an SVG, say).
  @visibleForTesting
  static Future<ui.Image?> draw(Uint8List bytes) async {
    ui.ImmutableBuffer? buffer;
    ui.ImageDescriptor? descriptor;
    try {
      buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
      descriptor = await ui.ImageDescriptor.encoded(buffer);
      final (w, h) = (descriptor.width, descriptor.height);
      final side = math.max(w, h);
      if (w < 1 || h < 1 || side > maxSide) return null;
      final scale = side > drawn ? drawn / side : 1.0;
      final codec = await descriptor.instantiateCodec(
        targetWidth: math.max(1, (w * scale).round()),
        targetHeight: math.max(1, (h * scale).round()),
      );
      final frame = await codec.getNextFrame();
      codec.dispose();
      return frame.image;
    } catch (_) {
      return null;
    } finally {
      descriptor?.dispose();
      buffer?.dispose();
    }
  }
}
