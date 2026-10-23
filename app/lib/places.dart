// Where items live, as the web vault's menu has it (passbundle-clients,
// vault-menu): Personal with the person's own folders, then each
// organization with its shared folders, nested by name (core/src/places.rs).
// The word "vault" is kept for the whole encrypted store (locking and
// unlocking it); every place an item lives is a folder.
import 'glyphs.dart';

/// What the vault's list shows: everything, favorites, the logins with
/// something to fix (Health), a kind, Personal, Personal's items in no
/// folder, one of the person's folders, an organization, or one of its
/// shared folders.
enum Filter {
  all,
  favorites,
  health,
  kind,
  personal,
  noFolder,
  folder,
  organization,
  collection,
}

/// The core's places (Core.places), read.
class Places {
  Places(Map<String, dynamic> json)
    : folders = ((json['folders'] as List?) ?? const [])
          .cast<Map<String, dynamic>>(),
      organizations = ((json['organizations'] as List?) ?? const [])
          .cast<Map<String, dynamic>>();

  static final empty = Places(const {});

  /// The person's own folders, nested: `{id, name, label, depth, parentId}`.
  final List<Map<String, dynamic>> folders;

  /// Each organization, `{id, name, collections}`, its shared folders
  /// nested as folders are, with the person's `grid` and `canCreate`.
  final List<Map<String, dynamic>> organizations;

  List<Map<String, dynamic>> collectionsOf(String? organizationId) =>
      ((organizations
                      .where((o) => o['id'] == organizationId)
                      .firstOrNull?['collections']
                  as List?) ??
              const [])
          .cast<Map<String, dynamic>>();

  Map<String, dynamic>? folder(String? id) =>
      folders.where((f) => f['id'] == id).firstOrNull;

  Map<String, dynamic>? organization(String? id) =>
      organizations.where((o) => o['id'] == id).firstOrNull;

  Map<String, dynamic>? collection(String? id) {
    for (final o in organizations) {
      for (final c in (o['collections'] as List).cast<Map<String, dynamic>>()) {
        if (c['id'] == id) return c;
      }
    }
    return null;
  }

  /// Whether the person may put new items in any of an organization's
  /// shared folders.
  bool canCreateIn(String organizationId) =>
      collectionsOf(organizationId).any((c) => c['canCreate'] == true);
}

/// One entry of the menu's Folders: an owner (Personal or an organization)
/// or a folder inside one, with how deep it sits, what picking it shows,
/// and whether it's one of the person's own folders (which they rename and
/// delete).
class PlaceEntry {
  const PlaceEntry({
    required this.label,
    required this.glyph,
    required this.depth,
    required this.filter,
    this.id,
    this.owner = false,
    this.ownFolder = false,
  });

  final String label;
  final String glyph;
  final int depth;
  final Filter filter;
  final String? id;

  /// Personal or an organization: a heading of its own folders.
  final bool owner;

  /// One of the person's own folders.
  final bool ownFolder;

  /// Every place, in the menu's order: Personal, its folders (and "No
  /// folder" when there are any), then each organization and its shared
  /// folders.
  static List<PlaceEntry> all(Places places) => [
    const PlaceEntry(
      label: 'Personal',
      glyph: Glyphs.person,
      depth: 0,
      filter: Filter.personal,
      owner: true,
    ),
    for (final f in places.folders)
      PlaceEntry(
        label: f['label'] as String,
        glyph: Glyphs.folder,
        depth: 1 + (f['depth'] as int),
        filter: Filter.folder,
        id: f['id'] as String,
        ownFolder: true,
      ),
    if (places.folders.isNotEmpty)
      const PlaceEntry(
        label: 'No folder',
        glyph: Glyphs.folder,
        depth: 1,
        filter: Filter.noFolder,
      ),
    for (final o in places.organizations) ...[
      PlaceEntry(
        label: o['name'] as String,
        glyph: Glyphs.building,
        depth: 0,
        filter: Filter.organization,
        id: o['id'] as String,
        owner: true,
      ),
      for (final c in (o['collections'] as List).cast<Map<String, dynamic>>())
        PlaceEntry(
          label: c['label'] as String,
          glyph: Glyphs.sharedFolder,
          depth: 1 + (c['depth'] as int),
          filter: Filter.collection,
          id: c['id'] as String,
        ),
    ],
  ];
}

/// Whether an item shows under a filter. A template an MSP keeps up to date
/// in a client's Templates shared folder shows only there. Health shows the
/// logins in [health] (core/src/health.rs).
bool shows(
  Map<String, dynamic> item,
  Filter filter,
  String? id, {
  Map<String, Map<String, dynamic>> health = const {},
}) {
  if (item['mspCopy'] == true && filter != Filter.collection) return false;
  return switch (filter) {
    Filter.all => true,
    Filter.favorites => item['favorite'] == true,
    Filter.health => health.containsKey(item['id']),
    Filter.kind => item['kind'] == id,
    Filter.personal => item['organizationId'] == null,
    Filter.noFolder =>
      item['organizationId'] == null && item['folderId'] == null,
    Filter.folder => item['folderId'] == id,
    Filter.organization => item['organizationId'] == id,
    Filter.collection =>
      ((item['collectionIds'] as List?) ?? const []).contains(id),
  };
}

/// Where an item is, said as the item's page says it: "Personal › Work",
/// "Contoso › IT".
String whereIs(Map<String, dynamic> item, Places places) {
  final org = item['organization'] as String?;
  if (org != null) {
    final collections =
        ((item['collectionIds'] as List?) ?? const [])
            .map((c) => places.collection(c as String)?['name'] as String?)
            .whereType<String>()
            .toList()
          ..sort((a, b) => a.toLowerCase().compareTo(b.toLowerCase()));
    return collections.isEmpty ? org : '$org › ${collections.join(', ')}';
  }
  final folder = item['folder'] as String?;
  return folder == null ? 'Personal' : 'Personal › $folder';
}

/// What a login has to fix, said briefly: "Weak password · used 3 times".
String healthIssues(Map<String, dynamic> issues) {
  final reused = (issues['reused'] as num?)?.toInt() ?? 0;
  return [
    if (issues['weak'] == true) 'Weak password',
    if (reused > 0)
      reused == 1 ? 'Password used twice' : 'Password used ${reused + 1} times',
    if (issues['unsecured'] == true) 'Unsecured website',
  ].join(' · ');
}
