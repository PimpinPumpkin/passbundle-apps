// A shared folder's icon and color, as the web vault and the extension draw
// them (passbundle-clients, libs/common/src/vault/keykeeper/folder-looks.ts;
// docs/core.md, "Folder icons"): one of our flat glyphs in one of the tiles'
// tones. Anyone who can rename the folder picks them in the web vault; the
// core opens them with the organization's key (core/src/places.rs) and the
// menu shows the folder as a tile. A name this app doesn't know still shows:
// the folder glyph, or gray.
import 'package:flutter/material.dart';

import 'glyphs.dart';
import 'theme.dart';

abstract final class FolderLooks {
  /// The icons, by the name the web vault stores: the categories' glyphs and
  /// passbundle.com's (site/icons.mjs in Passbundle), the same paths.
  static const icons = <String, String>{
    'folder': Glyphs.folder,
    'people':
        'M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z M3 20c.8-3.4 3-5 6-5s5.2 1.6 6 5 M16 4.5a3.5 3.5 0 0 1 0 6.5 M18 15c1.6.7 2.6 2.4 3 5',
    'bank': Glyphs.bank,
    'coins':
        'M4 7c0 1.7 3.6 3 8 3s8-1.3 8-3-3.6-3-8-3-8 1.3-8 3Z M4 7v5c0 1.7 3.6 3 8 3s8-1.3 8-3V7 M4 12v5c0 1.7 3.6 3 8 3s8-1.3 8-3v-5',
    'card': Glyphs.card,
    'key': Glyphs.key,
    'lock': 'M6 11h12v10H6Z M8.5 11V8a3.5 3.5 0 0 1 7 0v3 M12 15v2',
    'shield': Glyphs.health,
    'server': Glyphs.server,
    'terminal': Glyphs.terminal,
    'code': 'M8 7l-5 5 5 5 M16 7l5 5-5 5 M13.5 5l-3 14',
    'globe': Glyphs.globe,
    'browser': 'M3 5h18v14H3Z M3 9h18 M6 7h.01 M9 7h.01',
    'envelope': Glyphs.envelope,
    'chat': 'M4 5h16v11H9l-5 4Z M8 9.5h8 M8 12.5h5',
    'phone': 'M8 3h8v18H8Z M11 17.5h2',
    'laptop': 'M5 5h14v10H5Z M2.5 19h19 M4 15l-1.5 4 M20 15l1.5 4',
    'desktop': 'M3 4h18v12H3Z M8 20h8 M12 16v4',
    'wifi': Glyphs.wifi,
    'identity': Glyphs.identity,
    'license': Glyphs.license,
    'note': Glyphs.note,
    'star': Glyphs.star,
    'lifebuoy':
        'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8Z M5.6 5.6l3.6 3.6 M14.8 14.8l3.6 3.6 M18.4 5.6l-3.6 3.6 M9.2 14.8l-3.6 3.6',
  };

  /// The colors: the tiles' jewel tones (theme.dart), light and dark.
  static const tones = <String, (Color, Color)>{
    'sapphire': (Color(0xFF2448B8), Color(0xFF93ADFF)),
    'aquamarine': (Color(0xFF1C7FB8), Color(0xFF7CC4EF)),
    'turquoise': (Color(0xFF08788A), Color(0xFF62D3E3)),
    'emerald': (Color(0xFF0E7A47), Color(0xFF6FDBA0)),
    'gold': (Color(0xFF946A0C), Color(0xFFECC25E)),
    'topaz': (Color(0xFFA9600C), Color(0xFFF2B766)),
    'garnet': (Color(0xFFA8291F), Color(0xFFFF9D92)),
    'ruby': (Color(0xFFB01E54), Color(0xFFFF92B8)),
    'amethyst': (Color(0xFF6B30B8), Color(0xFFC6A3FF)),
    'onyx': (Color(0xFF353B4A), Color(0xFFC3C9D6)),
  };

  /// A folder's look, as the core gives it (`{icon, tone}`), as a jewel for
  /// its tile; null for the plain folder, which keeps its glyph.
  static Jewel? jewel(Object? look) {
    if (look is! Map) return null;
    final icon = look['icon'];
    final tone = look['tone'];
    if (icon is! String || tone is! String) return null;
    final (light, dark) = tones[tone] ?? tones['onyx']!;
    return Jewel(icons[icon] ?? Glyphs.folder, light, dark);
  }
}
