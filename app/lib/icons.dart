// Websites' icons on logins' tiles, as the web vault and the extension show
// them: from the server's icon service, through the core, which keeps them
// on this device for a while (core/src/icons.rs). A few at a time, once
// each, and none at all when Settings turns them off.
import 'dart:async';
import 'dart:collection';
import 'dart:io';

import 'core.dart';

/// Whether logins show their websites' icons (Settings, "Show website
/// icons"): read once from the core, and told when it changes.
final siteIcons = SiteIcons();

class SiteIcons {
  /// Each host's icon file, or null for none, once known.
  final _known = <String, String?>{};
  final _waiting = <String, Completer<String?>>{};
  final _queue = Queue<String>();
  int _running = 0;

  /// Bumped when icons are turned on or off, so tiles ask again.
  final changes = StreamController<void>.broadcast();

  bool? _on;
  bool get on => _on ??= Core().appearance()['showIcons'] != false;

  void turn(bool on) {
    _on = on;
    _known.clear();
    changes.add(null);
  }

  /// What's known for a host now, without waiting: the file, null for none,
  /// or [unknown].
  Object? now(String host) => _known.containsKey(host) ? _known[host] : unknown;

  static const unknown = Object();

  /// A host's icon file, or null: asked of the core four at a time.
  Future<String?> of(String host) {
    if (!on) return Future.value(null);
    if (_known.containsKey(host)) return Future.value(_known[host]);
    final waiting = _waiting[host];
    if (waiting != null) return waiting.future;
    final made = Completer<String?>();
    _waiting[host] = made;
    _queue.add(host);
    _next();
    return made.future;
  }

  void _next() {
    while (_running < 4 && _queue.isNotEmpty) {
      final host = _queue.removeFirst();
      _running++;
      Core.icon(host)
          .then<String?>(
            (path) => path != null && File(path).existsSync() ? path : null,
            onError: (Object _) => null,
          )
          .then((path) {
            _known[host] = path;
            _waiting.remove(host)?.complete(path);
            _running--;
            _next();
          });
    }
  }
}
