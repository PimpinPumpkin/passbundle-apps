// Where items live, as the web vault's menu has it (passbundle-clients,
// vault-menu): Personal with the person's own folders, then each
// organization with its shared folders, nested by name (core/src/places.rs).
// The word "vault" is kept for the whole encrypted store (locking and
// unlocking it); every place an item lives is a folder.
import 'folder_looks.dart';
import 'glyphs.dart';
import 'theme.dart';

/// What the vault's list shows: everything, favorites, what's expired or
/// expires within 30 days (Expiring soon), the person's own templates and
/// their organizations' (Item templates), the items with something to fix
/// (Health), a kind, Personal, Personal's items in no folder, one of the
/// person's folders, an organization, or one of its shared folders.
enum Filter {
  all,

  /// Every unlocked account's items, with several accounts signed in
  /// (accounts.dart, "All accounts").
  everyAccount,
  favorites,
  expiring,

  /// Custom templates, not an MSP's copies (the meta repo's DESIGN.md, "The
  /// Item templates view"): templates are in no other list but their own
  /// folder's or shared folder's, the archive, and the trash.
  templates,
  health,
  kind,
  personal,
  noFolder,
  folder,
  organization,
  collection,

  /// Archived items, out of the lists, search, and filling (the core's
  /// `Vault::archived`).
  archive,

  /// Items in the trash, in no other list, search, count, or filling (the
  /// core's `Vault::trashed`): restored, or deleted for good, from here.
  trash,
}

/// The core's places (Core.places), read.
class Places {
  Places(Map<String, dynamic> json)
    : folders = ((json['folders'] as List?) ?? const [])
          .cast<Map<String, dynamic>>(),
      organizations = ((json['organizations'] as List?) ?? const [])
          .cast<Map<String, dynamic>>(),
      newItems =
          (json['newItems'] as Map?)?.cast<String, dynamic>() ?? const {};

  static final empty = Places(const {});

  /// Where a new item may go, as the core has it for every screen that
  /// makes one (core/src/app.rs, `new_item_places`): `{personal,
  /// keepsItemsOut, start}`.
  final Map<String, dynamic> newItems;

  /// Personal and the person's own folders take new items: no organization
  /// keeps items out of Personal. Otherwise only the shared folders with
  /// `canCreate` do.
  bool get newInPersonal => newItems['personal'] != false;

  /// The organization that keeps items out of Personal, by name.
  String? get keepsItemsOut => newItems['keepsItemsOut'] as String?;

  /// Where a new item starts when no place shown takes it: a shared folder,
  /// as its organization's id and its own, where an organization keeps
  /// items out of Personal; null for Personal.
  (String, String)? get newItemStart => switch (newItems['start']) {
    {
      'organizationId': final String org,
      'collectionId': final String folder,
    } =>
      (org, folder),
    _ => null,
  };

  /// Nowhere takes a new item: an organization keeps items out of Personal,
  /// and the person has no shared folder they can add to.
  bool get nowhereNew => !newInPersonal && newItemStart == null;

  /// What the screens say then, in the import's words (transfer.dart).
  String get nowhereWords =>
      '${keepsItemsOut ?? 'Your organization'} keeps items out of Personal, and you have no shared folder you can add to. Ask an admin for one, then try again.';

  /// The person's own folders, nested: `{id, name, label, depth, parentId}`.
  final List<Map<String, dynamic>> folders;

  /// Each organization, `{id, name, collections}`, its shared folders
  /// nested as folders are, with the person's `grid` and `canCreate`, and
  /// each one's `look` (its icon and color, or null; folder_looks.dart).
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
    this.look,
  });

  final String label;
  final String glyph;

  /// A shared folder's own icon and color, as a jewel for its tile; null
  /// keeps the plain glyph.
  final Jewel? look;
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
          look: FolderLooks.jewel(c['look']),
        ),
    ],
  ];
}

/// An item's badge when it has expired or expires within 30 days
/// (core/src/expiration.rs): "Expires in 12 days", "Expired 5 days ago";
/// null otherwise. Cards expire with their expiry month.
String? expiringLabel(Map<String, dynamic> item) {
  final expiry = item['expiry'];
  if (expiry is! Map || expiry['state'] == 'later') return null;
  return expiry['label'] as String?;
}

/// Whether an item's expiry is past (its badge's color says so).
bool hasExpired(Map<String, dynamic> item) =>
    (item['expiry'] as Map?)?['state'] == 'expired';

/// Whether an item shows under a filter. A template an MSP keeps up to date
/// in a client's Templates shared folder shows only there, and in the
/// trash, which lists everything in it. The person's own templates and
/// their organizations' show under Item templates, in their own folder or
/// shared folder, and in the archive and the trash. Health shows the items
/// in [health] (core/src/health.rs and expiration.rs).
bool shows(
  Map<String, dynamic> item,
  Filter filter,
  String? id, {
  Map<String, Map<String, dynamic>> health = const {},
}) {
  if (item['mspCopy'] == true &&
      filter != Filter.collection &&
      filter != Filter.trash) {
    return false;
  }
  final template = isOwnTemplate(item);
  if (template &&
      !const {
        Filter.templates,
        Filter.folder,
        Filter.collection,
        Filter.archive,
        Filter.trash,
      }.contains(filter)) {
    return false;
  }
  return switch (filter) {
    Filter.templates => template,
    Filter.all || Filter.everyAccount => true,
    Filter.favorites => item['favorite'] == true,
    Filter.expiring => expiringLabel(item) != null,
    Filter.health => health.containsKey(item['id']),
    Filter.kind => item['kind'] == id,
    Filter.personal => item['organizationId'] == null,
    Filter.noFolder =>
      item['organizationId'] == null && item['folderId'] == null,
    Filter.folder => item['folderId'] == id,
    Filter.organization => item['organizationId'] == id,
    Filter.collection =>
      ((item['collectionIds'] as List?) ?? const []).contains(id),
    Filter.archive => item['archived'] == true,
    Filter.trash => item['deleted'] == true,
  };
}

/// One of the person's own templates or their organizations', not a copy
/// an MSP keeps up to date (core/src/templates.rs, `is_own_template`).
bool isOwnTemplate(Map<String, dynamic> item) =>
    item['isTemplate'] == true &&
    item['mspCopy'] != true &&
    item['kind'] == 'note';

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

/// What an item has to fix, said briefly: "Weak password · used 3 times",
/// "Expires in 5 days". Without [expiry], when it expires is left out (a
/// badge beside it says that).
String healthIssues(Map<String, dynamic> issues, {bool expiry = true}) {
  final reused = (issues['reused'] as num?)?.toInt() ?? 0;
  return [
    if (issues['weak'] == true) 'Weak password',
    if (reused > 0)
      reused == 1 ? 'Password used twice' : 'Password used ${reused + 1} times',
    if (issues['unsecured'] == true) 'Unsecured website',
    if (expiry)
      if (issues['expires'] case final String expires) expires,
  ].join(' · ');
}

/// Whether what an item has to fix is in one of Health's reports: `weak`,
/// `reused`, `unsecured`, or `expiring`.
bool inReport(Map<String, dynamic> issues, String report) => switch (report) {
  'weak' => issues['weak'] == true,
  'reused' => ((issues['reused'] as num?)?.toInt() ?? 0) > 0,
  'unsecured' => issues['unsecured'] == true,
  'expiring' => issues['expires'] != null,
  _ => true,
};

/// Whether a login is at risk (core/src/health.rs): a weak or reused
/// password, or a website saved with http://. What "Change password" is for.
bool atRisk(Map<String, dynamic>? issues) =>
    issues != null &&
    (issues['weak'] == true ||
        ((issues['reused'] as num?)?.toInt() ?? 0) > 0 ||
        issues['unsecured'] == true);
