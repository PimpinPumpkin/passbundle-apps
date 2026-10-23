// The vault: a sidebar (all items, favorites, folders, organizations), the
// list with search, and the item, side by side on a wide window. On a phone
// (or a phone-wide window), the vault is a tab beside Settings: its kinds as
// colored tiles, then favorites, folders, organizations, and every item, and
// an item opens on a page of its own. Copying goes through the core, so a
// password never passes through Dart; secrets reach the screen only while
// revealed.
import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'editor.dart';
import 'glyphs.dart';
import 'settings.dart';
import 'theme.dart';
import 'unlock_offer.dart';
import 'updates.dart';
import 'widgets.dart';

enum Filter { all, favorites, folder, organization, kind }

class VaultScreen extends StatefulWidget {
  const VaultScreen({
    super.key,
    required this.core,
    required this.email,
    this.server,
    required this.mode,
    required this.onMode,
    required this.onLock,
    required this.onSignInAgain,
    this.syncOnOpen = true,
    this.offerBiometric = false,
  });

  final Core core;
  final String email;
  final String? server;
  final ThemeMode mode;
  final ValueChanged<ThemeMode> onMode;
  final VoidCallback onLock;

  /// Signed out on the server: sign in again to get changes.
  final VoidCallback onSignInAgain;
  final bool syncOnOpen;

  /// Just opened with the master password: a phone may offer biometric
  /// unlock for next time (unlock_offer.dart).
  final bool offerBiometric;

  @override
  State<VaultScreen> createState() => VaultScreenState();
}

class VaultScreenState extends State<VaultScreen> {
  String query = '';
  Filter filter = Filter.all;
  String? filterId;
  String? selected;
  List<Map<String, dynamic>> all = [];
  Map<String, dynamic> places = {'folders': [], 'organizations': []};
  bool syncing = false;
  bool signedOut = false;

  /// On a phone: the vault (0) or Settings (1).
  int tab = 0;

  /// On a phone, the search field, kept across the tabs.
  final search = TextEditingController();

  /// Coming back to the app: a sign-in autofill offered to save may be
  /// waiting. Looked for after the next frame, since coming back can lock the
  /// app first (main.dart), and then it waits for the vault after unlocking.
  late final AppLifecycleListener lifecycle = AppLifecycleListener(
    onShow: () {
      WidgetsBinding.instance.addPostFrameCallback((_) => saveOffered());
      WidgetsBinding.instance.scheduleFrame();
    },
  );

  @override
  void initState() {
    super.initState();
    reload();
    lifecycle;
    updates?.addListener(updateFound);
    // Read now, before the first sync has the core.
    final offer = widget.offerBiometric && mayOfferBiometric(widget.core);
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      if (widget.syncOnOpen) sync();
      updateFound();
      // A sign-in to save opens the editor; the offer waits for another time.
      final saving = await saveOffered();
      if (!mounted) return;
      if (saving || !offer) return;
      final method = await offerBiometricUnlock(context);
      if (!mounted) return;
      if (method != null) showMessage(context, 'Unlocking with $method is on.');
    });
  }

  @override
  void dispose() {
    updates?.removeListener(updateFound);
    lifecycle.dispose();
    search.dispose();
    super.dispose();
  }

  /// A newer PassBundle the check on start found: a quiet notice, once (updates.dart).
  void updateFound() {
    if (mounted) showUpdateNotice(context);
  }

  /// A sign-in Android's autofill offered to save: the editor, filled in.
  /// True if there was one.
  Future<bool> saveOffered() async {
    if (!mounted) return false;
    final save = await Core.pendingSave();
    if (save == null || !mounted) return false;
    // A new password for a login it has comes with that login's id.
    unawaited(edit(save['id'] as String?, save));
    return true;
  }

  /// Reads the vault again from the core (never while a slow call runs).
  void reload() {
    if (busy.value) return;
    setState(() {
      all = widget.core.list(query);
      places = widget.core.places();
      if (selected != null && widget.core.item(selected!) == null) {
        selected = null;
      }
    });
  }

  Future<void> sync() async {
    if (syncing) return;
    setState(() => syncing = true);
    final answer = await whileBusy(Core.sync);
    if (!mounted) return;
    setState(() {
      syncing = false;
      signedOut = answer.signedOut;
    });
    if (!answer.ok && !answer.signedOut) {
      showMessage(context, answer.error ?? 'Couldn\'t sync.');
    }
    reload();
  }

  List<Map<String, dynamic>> get shown => switch (filter) {
    Filter.all => all,
    Filter.favorites => all.where((i) => i['favorite'] == true).toList(),
    Filter.folder => all.where((i) => i['folderId'] == filterId).toList(),
    Filter.organization =>
      all.where((i) => i['organizationId'] == filterId).toList(),
    Filter.kind => all.where((i) => i['kind'] == filterId).toList(),
  };

  List<Map<String, dynamic>> get folders =>
      (places['folders'] as List).cast<Map<String, dynamic>>();

  List<Map<String, dynamic>> get organizations =>
      (places['organizations'] as List).cast<Map<String, dynamic>>();

  /// What the list shows, by name, for a phone's header.
  String? get placeName {
    String? named(List<Map<String, dynamic>> places) => places
        .where((p) => p['id'] == filterId)
        .map((p) => p['name'] as String)
        .firstOrNull;
    return switch (filter) {
      Filter.all => null,
      Filter.favorites => 'Favorites',
      Filter.folder => named(folders),
      Filter.organization => named(organizations),
      Filter.kind => ItemKind.of(filterId ?? '').many,
    };
  }

  void choose(Filter f, [String? id]) => setState(() {
    filter = f;
    filterId = id;
  });

  Future<void> edit([String? id, Map<String, dynamic>? prefill]) async {
    final wide = MediaQuery.sizeOf(context).width >= 900;
    // An item that asks for the master password gets it before its editor
    // opens, so the editor shows its secrets, not blanks (Android's "Save
    // to PassBundle" opens the editor straight away).
    if (id != null) {
      final item = widget.core.item(id);
      if (item != null &&
          item['reprompt'] == true &&
          item['mayUse'] != true &&
          !await askForMasterPassword(context, id, item['name'] as String)) {
        return;
      }
      if (!mounted) return;
    }
    final saved = await openEditor(
      context,
      core: widget.core,
      places: places,
      id: id,
      prefill: prefill,
      asDialog: wide,
    );
    if (saved == null || !mounted) return;
    reload();
    setState(() => selected = saved.isEmpty ? selected : saved);
    showMessage(context, id == null ? 'Saved' : 'Saved the changes');
  }

  /// Moves an item to the trash, once the person says so: true if it went.
  Future<bool> trash(String id, String name) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Move $name to the trash?'),
        content: const Text(
          'You can restore it from the trash in the web vault.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Move to trash'),
          ),
        ],
      ),
    );
    if (sure != true || !mounted) return false;
    final answer = await whileBusy(() => Core.trash(id));
    if (!mounted) return false;
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t move it to the trash.');
      return false;
    }
    setState(() => selected = null);
    reload();
    return true;
  }

  /// Makes a folder, then shows it.
  Future<void> newFolder() async {
    final name = await askFolderName(context, 'New folder');
    if (name == null || !mounted) return;
    final answer = await whileBusy(() => Core.createFolder(name));
    if (!mounted) return;
    reload();
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t make the folder.');
      return;
    }
    choose(Filter.folder, answer.body['id'] as String?);
  }

  Future<void> renameFolder(String id, String name) async {
    final renamed = await askFolderName(context, 'Rename folder', name);
    if (renamed == null || renamed == name || !mounted) return;
    final answer = await whileBusy(() => Core.renameFolder(id, renamed));
    if (!mounted) return;
    reload();
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t rename the folder.');
    }
  }

  /// Deletes a folder; its items stay, in no folder.
  Future<void> deleteFolder(String id, String name) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Delete the folder $name?'),
        content: const Text('Its items stay in your vault, in no folder.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Delete folder'),
          ),
        ],
      ),
    );
    if (sure != true || !mounted) return;
    final answer = await whileBusy(() => Core.deleteFolder(id));
    if (!mounted) return;
    reload();
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t delete the folder.');
      return;
    }
    if (filter == Filter.folder && filterId == id) choose(Filter.all);
  }

  /// On a phone, an item opens on its own page.
  void open(String id) => Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => ItemPane(
        core: widget.core,
        id: id,
        phone: true,
        onEdit: () => edit(id),
        onTrash: (name) async {
          if (await trash(id, name) && mounted) Navigator.of(context).pop();
        },
      ),
    ),
  );

  Widget? get signedOutBanner => signedOut
      ? MaterialBanner(
          content: const Text(
            'You\'ve been signed out on the server. Sign in again to get changes.',
          ),
          leading: const Icon(Icons.cloud_off_outlined),
          actions: [
            TextButton(
              onPressed: widget.onSignInAgain,
              child: const Text('Sign in'),
            ),
          ],
        )
      : null;

  @override
  Widget build(BuildContext context) {
    final wide = MediaQuery.sizeOf(context).width >= 900;
    if (!wide) return phone(context);
    final pane = selected == null
        ? null
        : ItemPane(
            key: ValueKey(selected),
            core: widget.core,
            id: selected!,
            onEdit: () => edit(selected),
            onTrash: (name) => trash(selected!, name),
          );
    return Scaffold(
      body: Column(
        children: [
          ?signedOutBanner,
          Expanded(
            child: Row(
              children: [
                Sidebar(
                  email: widget.email,
                  filter: filter,
                  filterId: filterId,
                  places: places,
                  items: all,
                  onChoose: choose,
                  onNewFolder: newFolder,
                  onRenameFolder: renameFolder,
                  onDeleteFolder: deleteFolder,
                  mode: widget.mode,
                  onMode: widget.onMode,
                  onLock: widget.onLock,
                ),
                SizedBox(
                  width: 360,
                  child: ItemList(
                    title: placeName ?? 'All items',
                    items: shown,
                    selected: selected,
                    syncing: syncing,
                    onSearch: (q) {
                      query = q;
                      reload();
                    },
                    onNew: () => edit(),
                    onSync: sync,
                    onSelect: (id) => setState(() => selected = id),
                  ),
                ),
                Expanded(
                  child:
                      pane ??
                      Center(
                        child: Text(
                          'Choose an item',
                          style: TextStyle(color: context.colors.muted),
                        ),
                      ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// On a phone: the vault and Settings, as tabs, and New item over the
  /// vault. Back goes back from a search, a kind or folder, or Settings,
  /// before it leaves the app.
  Widget phone(BuildContext context) {
    final home = tab == 0 && filter == Filter.all && query.isEmpty;
    return PopScope(
      canPop: home,
      onPopInvokedWithResult: (popped, _) {
        if (popped) return;
        if (tab != 0) {
          setState(() => tab = 0);
        } else if (query.isNotEmpty) {
          clearSearch();
        } else {
          choose(Filter.all);
        }
      },
      child: Scaffold(
        body: SafeArea(
          bottom: false,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              ?signedOutBanner,
              Expanded(
                child: IndexedStack(
                  index: tab,
                  children: [
                    phoneVault(context),
                    // Built each time it's chosen, so it shows what's so now.
                    tab == 1
                        ? SettingsPage(
                            email: widget.email,
                            server: widget.server,
                            mode: widget.mode,
                            onMode: widget.onMode,
                          )
                        : const SizedBox.shrink(),
                  ],
                ),
              ),
            ],
          ),
        ),
        floatingActionButton: tab == 0
            ? FloatingActionButton(
                tooltip: 'New item',
                onPressed: () => edit(),
                child: Glyph(
                  Glyphs.plus,
                  strokeWidth: 2.2,
                  color: Theme.of(context).colorScheme.onPrimary,
                ),
              )
            : null,
        bottomNavigationBar: NavigationBar(
          selectedIndex: tab,
          onDestinationSelected: (i) => setState(() => tab = i),
          // The web vault's glyphs: all items, and the gear.
          destinations: const [
            NavigationDestination(
              icon: Glyph(Glyphs.grid, size: 22),
              label: 'Vault',
            ),
            NavigationDestination(
              icon: Glyph(Glyphs.settings, size: 22),
              label: 'Settings',
            ),
          ],
        ),
      ),
    );
  }

  void clearSearch() {
    search.clear();
    query = '';
    reload();
  }

  Widget phoneVault(BuildContext context) {
    final c = context.colors;
    final filtered = filter != Filter.all;
    final folder = filter == Filter.folder
        ? folders.where((f) => f['id'] == filterId).firstOrNull
        : null;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        PageTitle(
          placeName ?? 'Vault',
          leading: filtered
              ? IconButton(
                  tooltip: 'Back to the vault',
                  icon: const Icon(Icons.arrow_back_rounded),
                  onPressed: () => choose(Filter.all),
                )
              : null,
          actions: [
            if (folder != null)
              FolderMenu(
                name: folder['name'] as String,
                color: c.muted,
                large: true,
                onRename: () => renameFolder(
                  folder['id'] as String,
                  folder['name'] as String,
                ),
                onDelete: () => deleteFolder(
                  folder['id'] as String,
                  folder['name'] as String,
                ),
              ),
            if (syncing)
              const SizedBox(
                width: 48,
                height: 48,
                child: Center(
                  child: SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  ),
                ),
              )
            else
              IconButton(
                tooltip: 'Sync',
                icon: const Icon(Icons.sync_rounded),
                onPressed: sync,
              ),
            IconButton(
              tooltip: 'Lock',
              icon: const Icon(Icons.lock_outline_rounded),
              onPressed: widget.onLock,
            ),
          ],
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 4, 16, 8),
          child: TextField(
            controller: search,
            textInputAction: TextInputAction.search,
            onChanged: (q) {
              query = q;
              reload();
            },
            decoration: InputDecoration(
              hintText: filtered
                  ? 'Search ${placeName ?? 'your vault'}'
                  : 'Search your vault',
              prefixIcon: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 14),
                child: Glyph(Glyphs.search, size: 20, color: c.muted),
              ),
              prefixIconConstraints: const BoxConstraints(minWidth: 48),
              suffixIcon: query.isEmpty
                  ? null
                  : IconButton(
                      tooltip: 'Clear the search',
                      icon: const Icon(Icons.close_rounded),
                      onPressed: clearSearch,
                    ),
              contentPadding: const EdgeInsets.symmetric(vertical: 14),
              border: OutlineInputBorder(
                borderRadius: BorderRadius.circular(14),
                borderSide: BorderSide(color: c.line),
              ),
              enabledBorder: OutlineInputBorder(
                borderRadius: BorderRadius.circular(14),
                borderSide: BorderSide(color: c.line),
              ),
              focusedBorder: OutlineInputBorder(
                borderRadius: BorderRadius.circular(14),
                borderSide: BorderSide(
                  color: Theme.of(context).colorScheme.primary,
                  width: 1.6,
                ),
              ),
            ),
          ),
        ),
        Expanded(
          child: RefreshIndicator(
            onRefresh: sync,
            child: CustomScrollView(
              physics: const AlwaysScrollableScrollPhysics(),
              slivers: phoneSlivers(context),
            ),
          ),
        ),
      ],
    );
  }

  List<Widget> phoneSlivers(BuildContext context) {
    final c = context.colors;
    // Room under the last row for New item, which floats over it.
    const end = SliverToBoxAdapter(child: SizedBox(height: 96));
    if (filter != Filter.all || query.isNotEmpty) {
      final list = shown;
      return [
        if (list.isEmpty)
          SliverFillRemaining(
            hasScrollBody: false,
            child: Center(
              child: Text(
                query.isEmpty ? 'Nothing here' : 'Nothing matches',
                style: TextStyle(color: c.muted),
              ),
            ),
          )
        else ...[
          const SliverToBoxAdapter(child: SizedBox(height: 8)),
          itemCard(list),
          end,
        ],
      ];
    }
    final favorites = all.where((i) => i['favorite'] == true).toList();
    Widget heading(String text, [Widget? trailing]) => SliverToBoxAdapter(
      child: SectionHeading(
        text,
        padding: const EdgeInsets.fromLTRB(20, 12, 8, 2),
        height: 44,
        trailing: trailing,
      ),
    );
    int count(bool Function(Map<String, dynamic>) test) =>
        all.where(test).length;
    return [
      SliverPadding(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
        sliver: SliverToBoxAdapter(
          child: Categories(
            counts: {
              for (final k in ItemKind.all)
                k.kind: count((i) => i['kind'] == k.kind),
            },
            onChoose: (kind) => choose(Filter.kind, kind),
          ),
        ),
      ),
      if (favorites.isNotEmpty) ...[heading('Favorites'), itemCard(favorites)],
      heading(
        'Folders',
        IconButton(
          tooltip: 'New folder',
          icon: const Icon(Icons.create_new_folder_outlined, size: 22),
          onPressed: newFolder,
        ),
      ),
      if (folders.isNotEmpty)
        placeCard([
          for (final f in folders)
            (
              Glyphs.folder,
              f['name'] as String,
              count((i) => i['folderId'] == f['id']),
              () => choose(Filter.folder, f['id'] as String),
            ),
        ]),
      if (organizations.isNotEmpty) ...[
        heading('Organizations'),
        placeCard([
          for (final o in organizations)
            (
              Glyphs.building,
              o['name'] as String,
              count((i) => i['organizationId'] == o['id']),
              () => choose(Filter.organization, o['id'] as String),
            ),
        ]),
      ],
      heading('All items'),
      if (all.isEmpty)
        SliverToBoxAdapter(
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
            child: Text(
              'Nothing here yet. New item adds one.',
              style: TextStyle(color: c.muted),
            ),
          ),
        )
      else
        itemCard(all),
      end,
    ];
  }

  /// Rows on a resting card, built as they scroll into view.
  Widget card(int count, Widget Function(BuildContext, int, BorderRadius) row) {
    const r = Radius.circular(16);
    final c = context.colors;
    return SliverPadding(
      padding: const EdgeInsets.symmetric(horizontal: 16),
      sliver: DecoratedSliver(
        decoration: BoxDecoration(
          color: c.pane,
          borderRadius: const BorderRadius.all(r),
          boxShadow: c.resting,
        ),
        sliver: SliverList.separated(
          itemCount: count,
          itemBuilder: (context, i) => row(
            context,
            i,
            BorderRadius.vertical(
              top: i == 0 ? r : Radius.zero,
              bottom: i == count - 1 ? r : Radius.zero,
            ),
          ),
          separatorBuilder: (_, _) => const Divider(indent: 68),
        ),
      ),
    );
  }

  Widget itemCard(List<Map<String, dynamic>> items) =>
      card(items.length, (context, i, radius) {
        final item = items[i];
        return PhoneItemRow(
          item: item,
          radius: radius,
          onTap: () => open(item['id'] as String),
        );
      });

  Widget placeCard(List<(String, String, int, VoidCallback)> places) => card(
    places.length,
    (context, i, radius) {
      final (icon, name, count, tap) = places[i];
      final c = context.colors;
      return InkWell(
        borderRadius: radius,
        onTap: tap,
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 56),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 12, 8),
            child: Row(
              children: [
                Container(
                  width: 40,
                  height: 40,
                  decoration: BoxDecoration(
                    color: c.tint,
                    borderRadius: BorderRadius.circular(11),
                  ),
                  alignment: Alignment.center,
                  child: Glyph(
                    icon,
                    size: 21,
                    color: Theme.of(context).colorScheme.primary,
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Text(
                    name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 16,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ),
                Text('$count', style: TextStyle(color: c.muted, fontSize: 14)),
                const SizedBox(width: 6),
                Glyph(Glyphs.chevron, size: 18, color: c.muted, strokeWidth: 2),
              ],
            ),
          ),
        ),
      );
    },
  );
}

void showMessage(BuildContext context, String text) =>
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(text), duration: const Duration(seconds: 3)),
    );

/// The vault's kinds on a phone, each its jewel tile with how many there
/// are: two to a row, or one when the names need the room (a large text
/// size, a narrow phone).
class Categories extends StatelessWidget {
  const Categories({super.key, required this.counts, required this.onChoose});

  final Map<String, int> counts;
  final ValueChanged<String> onChoose;

  static const _gap = 10.0;
  static const _name = TextStyle(
    fontFamily: passBundleFontFamily,
    fontSize: 15,
    fontWeight: FontWeight.w600,
  );
  static const _count = TextStyle(
    fontFamily: passBundleFontFamily,
    fontSize: 14,
  );

  double _width(String text, TextStyle style, TextScaler scaler) {
    final painter = TextPainter(
      text: TextSpan(text: text, style: style),
      textScaler: scaler,
      textDirection: TextDirection.ltr,
      maxLines: 1,
    )..layout();
    final width = painter.width;
    painter.dispose();
    return width;
  }

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final scaler = MediaQuery.textScalerOf(context);
      final widest = ItemKind.all
          .map((k) => _width(k.many, _name, scaler))
          .reduce(math.max);
      final most = counts.values.fold(0, math.max);
      // A tile: its padding, the jewel, the name, and the count.
      final needs =
          12 + 34 + 10 + widest + 8 + _width('$most', _count, scaler) + 12;
      final columns = needs <= (constraints.maxWidth - _gap) / 2 ? 2 : 1;
      final kinds = ItemKind.all;
      return Column(
        children: [
          for (var i = 0; i < kinds.length; i += columns) ...[
            if (i > 0) const SizedBox(height: _gap),
            Row(
              children: [
                for (var j = i; j < i + columns; j++) ...[
                  if (j > i) const SizedBox(width: _gap),
                  Expanded(
                    child: j < kinds.length
                        ? _tile(context, kinds[j])
                        : const SizedBox.shrink(),
                  ),
                ],
              ],
            ),
          ],
        ],
      );
    },
  );

  Widget _tile(BuildContext context, ItemKind kind) => RestingCard(
    radius: 14,
    child: InkWell(
      onTap: () => onChoose(kind.kind),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 56),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
          child: Row(
            children: [
              JewelTile(jewel: kind, size: 34),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  kind.many,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: _name,
                ),
              ),
              const SizedBox(width: 8),
              Text(
                '${counts[kind.kind] ?? 0}',
                style: _count.copyWith(color: context.colors.muted),
              ),
            ],
          ),
        ),
      ),
    ),
  );
}

/// An item's row in a phone's list: its tile, name, and what tells it apart
/// (a username, say), and that it opens.
class PhoneItemRow extends StatelessWidget {
  const PhoneItemRow({
    super.key,
    required this.item,
    required this.radius,
    required this.onTap,
  });

  final Map<String, dynamic> item;
  final BorderRadius radius;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final subtitle = item['subtitle'] as String;
    return InkWell(
      borderRadius: radius,
      onTap: onTap,
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 64),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 10, 12, 10),
          child: Row(
            children: [
              Avatar(
                name: item['name'] as String,
                kind: item['kind'] as String,
                size: 40,
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      item['name'] as String,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        fontSize: 16,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    if (subtitle.isNotEmpty) ...[
                      const SizedBox(height: 2),
                      Text(
                        subtitle,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(color: c.muted, fontSize: 14),
                      ),
                    ],
                  ],
                ),
              ),
              if (item['favorite'] == true)
                const Padding(
                  padding: EdgeInsets.only(left: 6),
                  child: Glyph(
                    Glyphs.star,
                    size: 18,
                    color: favoriteEdge,
                    fill: favoriteColor,
                    strokeWidth: 1.6,
                    semanticLabel: 'Favorite',
                  ),
                ),
              const SizedBox(width: 6),
              Glyph(Glyphs.chevron, size: 18, color: c.muted, strokeWidth: 2),
            ],
          ),
        ),
      ),
    );
  }
}

/// A favorite's star, and its edge.
const favoriteColor = Color(0xFFE8B23A);
const favoriteEdge = Color(0xFFC9921B);

/// The menu on a computer, as the web vault has it: a card on the ground
/// with All items and Favorites, the kinds of item with their jewel tiles,
/// folders, and organizations, each with how many it holds; then who's
/// signed in, the theme, Settings, and Lock.
class Sidebar extends StatelessWidget {
  const Sidebar({
    super.key,
    this.width = 248,
    required this.email,
    required this.filter,
    required this.filterId,
    required this.places,
    required this.items,
    required this.onChoose,
    required this.onNewFolder,
    required this.onRenameFolder,
    required this.onDeleteFolder,
    required this.mode,
    required this.onMode,
    required this.onLock,
  });

  final double width;
  final String email;
  final Filter filter;
  final String? filterId;
  final Map<String, dynamic> places;

  /// Every item, for the counts beside each entry.
  final List<Map<String, dynamic>> items;
  final void Function(Filter, [String?]) onChoose;
  final VoidCallback onNewFolder;
  final void Function(String id, String name) onRenameFolder;
  final void Function(String id, String name) onDeleteFolder;
  final ThemeMode mode;
  final ValueChanged<ThemeMode> onMode;
  final VoidCallback onLock;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    int count(bool Function(Map<String, dynamic>) test) =>
        items.where(test).length;
    Widget entry({
      required Widget tile,
      required String label,
      required bool on,
      required VoidCallback tap,
      int? count,
      Widget? trailing,
    }) => Padding(
      padding: const EdgeInsets.symmetric(vertical: 1),
      child: Material(
        color: on ? c.tint : Colors.transparent,
        borderRadius: BorderRadius.circular(10),
        child: InkWell(
          borderRadius: BorderRadius.circular(10),
          onTap: tap,
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 38),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
              child: Row(
                children: [
                  SizedBox(width: 24, child: Center(child: tile)),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      label,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: 14,
                        fontWeight: on ? FontWeight.w700 : FontWeight.w500,
                      ),
                    ),
                  ),
                  if (count != null)
                    Text(
                      '$count',
                      style: TextStyle(color: c.muted, fontSize: 12),
                    ),
                  ?trailing,
                ],
              ),
            ),
          ),
        ),
      ),
    );
    Widget heading(String text, [Widget? trailing]) => Padding(
      padding: const EdgeInsets.fromLTRB(10, 14, 0, 4),
      child: Row(
        children: [
          Expanded(
            child: Semantics(
              header: true,
              child: Text(
                text.toUpperCase(),
                style: TextStyle(
                  color: c.muted,
                  fontSize: 11,
                  letterSpacing: 0.7,
                  fontWeight: FontWeight.w700,
                ),
              ),
            ),
          ),
          ?trailing,
        ],
      ),
    );
    Widget outline(String glyph) => Glyph(glyph, size: 18, color: c.muted);
    final folders = (places['folders'] as List).cast<Map<String, dynamic>>();
    final organizations = (places['organizations'] as List)
        .cast<Map<String, dynamic>>();
    return Container(
      width: width,
      margin: const EdgeInsets.fromLTRB(12, 12, 0, 12),
      decoration: BoxDecoration(
        color: c.pane,
        borderRadius: BorderRadius.circular(16),
        boxShadow: c.resting,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 18, 16, 10),
            child: Row(
              children: [
                // The mark on an emerald jewel, as the web vault's menu has it.
                const JewelTile(
                  jewel: Jewel.allItems,
                  size: 30,
                  child: Mark(size: 16, color: Colors.white),
                ),
                const SizedBox(width: 10),
                const Text(
                  'PassBundle',
                  style: TextStyle(
                    fontWeight: FontWeight.w700,
                    fontSize: 18,
                    letterSpacing: -0.2,
                  ),
                ),
              ],
            ),
          ),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.symmetric(horizontal: 8),
              children: [
                entry(
                  tile: const JewelTile(jewel: Jewel.allItems, size: 24),
                  label: 'All items',
                  on: filter == Filter.all,
                  tap: () => onChoose(Filter.all),
                  count: items.length,
                ),
                entry(
                  tile: const JewelTile(jewel: Jewel.favorites, size: 24),
                  label: 'Favorites',
                  on: filter == Filter.favorites,
                  tap: () => onChoose(Filter.favorites),
                  count: count((i) => i['favorite'] == true),
                ),
                heading('Categories'),
                for (final k in ItemKind.all)
                  entry(
                    tile: JewelTile(jewel: k, size: 24),
                    label: k.many,
                    on: filter == Filter.kind && filterId == k.kind,
                    tap: () => onChoose(Filter.kind, k.kind),
                    count: count((i) => i['kind'] == k.kind),
                  ),
                heading(
                  'Folders',
                  IconButton(
                    tooltip: 'New folder',
                    visualDensity: VisualDensity.compact,
                    iconSize: 18,
                    color: c.muted,
                    onPressed: onNewFolder,
                    icon: const Icon(Icons.add_rounded),
                  ),
                ),
                for (final f in folders)
                  entry(
                    tile: outline(Glyphs.folder),
                    label: f['name'] as String,
                    on: filter == Filter.folder && filterId == f['id'],
                    tap: () => onChoose(Filter.folder, f['id'] as String),
                    count: count((i) => i['folderId'] == f['id']),
                    trailing: Padding(
                      padding: const EdgeInsets.only(left: 4),
                      child: FolderMenu(
                        name: f['name'] as String,
                        color: c.muted,
                        onRename: () => onRenameFolder(
                          f['id'] as String,
                          f['name'] as String,
                        ),
                        onDelete: () => onDeleteFolder(
                          f['id'] as String,
                          f['name'] as String,
                        ),
                      ),
                    ),
                  ),
                if (organizations.isNotEmpty) heading('Organizations'),
                for (final o in organizations)
                  entry(
                    tile: outline(Glyphs.building),
                    label: o['name'] as String,
                    on: filter == Filter.organization && filterId == o['id'],
                    tap: () => onChoose(Filter.organization, o['id'] as String),
                    count: count((i) => i['organizationId'] == o['id']),
                  ),
                const SizedBox(height: 8),
              ],
            ),
          ),
          const Divider(indent: 14, endIndent: 14),
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 10, 16, 0),
            child: Text(
              email,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(color: c.muted, fontSize: 12),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 4, 8, 8),
            child: Row(
              children: [
                ThemeMenu(mode: mode, onMode: onMode, color: c.muted),
                SettingsButton(color: c.muted),
                const Spacer(),
                TextButton.icon(
                  onPressed: onLock,
                  icon: const Icon(Icons.lock_outline_rounded, size: 18),
                  label: const Text('Lock'),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The list beside the menu, on a computer: search and New, what it shows
/// and how many, and the items, the chosen one raised.
class ItemList extends StatelessWidget {
  const ItemList({
    super.key,
    required this.title,
    required this.items,
    required this.selected,
    required this.syncing,
    required this.onSearch,
    required this.onNew,
    required this.onSync,
    required this.onSelect,
  });

  /// What the list shows: All items, a kind, a folder.
  final String title;
  final List<Map<String, dynamic>> items;
  final String? selected;
  final bool syncing;
  final ValueChanged<String> onSearch;
  final VoidCallback onNew;
  final VoidCallback onSync;
  final ValueChanged<String> onSelect;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final dark = Theme.of(context).brightness == Brightness.dark;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 12, 4),
          child: Row(
            children: [
              Expanded(
                child: TextField(
                  onChanged: onSearch,
                  decoration: InputDecoration(
                    hintText: 'Search',
                    prefixIcon: Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 12),
                      child: Glyph(Glyphs.search, size: 18, color: c.muted),
                    ),
                    prefixIconConstraints: const BoxConstraints(minWidth: 42),
                  ),
                ),
              ),
              const SizedBox(width: 4),
              syncing
                  ? const SizedBox(
                      width: 44,
                      height: 44,
                      child: Center(
                        child: SizedBox(
                          width: 18,
                          height: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        ),
                      ),
                    )
                  : IconButton(
                      tooltip: 'Sync',
                      icon: const Icon(Icons.sync_rounded),
                      onPressed: onSync,
                    ),
              const SizedBox(width: 4),
              Tooltip(
                message: 'New item',
                excludeFromSemantics: true,
                child: FilledButton.icon(
                  onPressed: onNew,
                  style: FilledButton.styleFrom(
                    minimumSize: const Size(0, 44),
                    padding: const EdgeInsets.symmetric(horizontal: 14),
                  ),
                  icon: Glyph(
                    Glyphs.plus,
                    size: 18,
                    strokeWidth: 2.4,
                    color: Theme.of(context).colorScheme.onPrimary,
                  ),
                  label: const Text('New'),
                ),
              ),
            ],
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(22, 10, 18, 6),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.baseline,
            textBaseline: TextBaseline.alphabetic,
            children: [
              Expanded(
                child: Text(
                  title,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    fontSize: 22,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ),
              Text(
                items.length == 1 ? '1 item' : '${items.length} items',
                style: TextStyle(color: c.muted, fontSize: 13),
              ),
            ],
          ),
        ),
        Expanded(
          child: items.isEmpty
              ? Center(
                  child: Text('Nothing here', style: TextStyle(color: c.muted)),
                )
              : ListView.builder(
                  padding: const EdgeInsets.fromLTRB(10, 2, 10, 12),
                  itemCount: items.length,
                  itemBuilder: (context, i) {
                    final item = items[i];
                    final on = item['id'] == selected;
                    final subtitle = item['subtitle'] as String;
                    return Padding(
                      padding: const EdgeInsets.symmetric(vertical: 2),
                      // The chosen one rises off the ground.
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          borderRadius: BorderRadius.circular(12),
                          boxShadow: on
                              ? [
                                  BoxShadow(
                                    color: dark
                                        ? const Color(0xFF2F5247)
                                        : const Color(0xFFCFE3DA),
                                    spreadRadius: 1,
                                  ),
                                  ...c.lifted,
                                ]
                              : null,
                        ),
                        child: Material(
                          color: on ? c.pane : Colors.transparent,
                          borderRadius: BorderRadius.circular(12),
                          child: InkWell(
                            borderRadius: BorderRadius.circular(12),
                            onTap: () => onSelect(item['id'] as String),
                            child: Padding(
                              padding: const EdgeInsets.symmetric(
                                horizontal: 12,
                                vertical: 10,
                              ),
                              child: Row(
                                children: [
                                  Avatar(
                                    name: item['name'] as String,
                                    kind: item['kind'] as String,
                                    size: 40,
                                  ),
                                  const SizedBox(width: 12),
                                  Expanded(
                                    child: Column(
                                      crossAxisAlignment:
                                          CrossAxisAlignment.start,
                                      children: [
                                        Text(
                                          item['name'] as String,
                                          maxLines: 1,
                                          overflow: TextOverflow.ellipsis,
                                          style: const TextStyle(
                                            fontSize: 15,
                                            fontWeight: FontWeight.w600,
                                          ),
                                        ),
                                        if (subtitle.isNotEmpty)
                                          Text(
                                            subtitle,
                                            maxLines: 1,
                                            overflow: TextOverflow.ellipsis,
                                            style: TextStyle(
                                              color: c.muted,
                                              fontSize: 13,
                                            ),
                                          ),
                                      ],
                                    ),
                                  ),
                                  if (item['favorite'] == true)
                                    const Padding(
                                      padding: EdgeInsets.only(left: 6),
                                      child: Glyph(
                                        Glyphs.star,
                                        size: 16,
                                        color: favoriteEdge,
                                        fill: favoriteColor,
                                        strokeWidth: 1.6,
                                        semanticLabel: 'Favorite',
                                      ),
                                    ),
                                ],
                              ),
                            ),
                          ),
                        ),
                      ),
                    );
                  },
                ),
        ),
      ],
    );
  }
}

/// An item: beside the list on a computer, or on a phone a page of its own,
/// with a big tile, its name, and big copy buttons.
class ItemPane extends StatefulWidget {
  const ItemPane({
    super.key,
    required this.core,
    required this.id,
    required this.onEdit,
    required this.onTrash,
    this.phone = false,
  });

  final Core core;
  final String id;

  /// Opens the editor; done when it closes.
  final Future<void> Function() onEdit;
  final ValueChanged<String> onTrash;

  /// A page of its own, with Edit and Move to trash at the top.
  final bool phone;

  @override
  State<ItemPane> createState() => _ItemPaneState();
}

class _ItemPaneState extends State<ItemPane> {
  final revealed = <String, String>{};
  Map<String, dynamic>? item;
  Map<String, dynamic>? totp;
  Timer? ticker;

  @override
  void initState() {
    super.initState();
    item = widget.core.item(widget.id);
    tick();
    ticker = Timer.periodic(const Duration(seconds: 1), (_) => tick());
  }

  void tick() {
    if (busy.value || item?['hasTotp'] != true) return;
    setState(() => totp = widget.core.totp(widget.id));
  }

  @override
  void dispose() {
    ticker?.cancel();
    masterPassword.dispose();
    super.dispose();
  }

  /// Edits it, then shows what was saved.
  Future<void> edit() async {
    await widget.onEdit();
    if (!mounted) return;
    setState(() {
      item = widget.core.item(widget.id);
      revealed.clear();
    });
  }

  void toggle(String what) => setState(() {
    if (asksAgain()) return;
    if (revealed.remove(what) == null) {
      final secret = widget.core.reveal(widget.id, what);
      if (secret != null) revealed[what] = secret;
    }
  });

  /// Whether the item asks for the master password again now (it was given
  /// for another item since): then the pane asks before anything else.
  bool asksAgain() {
    final fresh = widget.core.item(widget.id);
    if (fresh == null || fresh['reprompt'] != true || fresh['mayUse'] == true) {
      return false;
    }
    item = fresh;
    revealed.clear();
    return true;
  }

  /// Downloads, opens, and saves an attachment to Downloads, through the core.
  Future<void> saveAttachment(String attachmentId, String name) async {
    if (asksAgain()) {
      setState(() {});
      return;
    }
    final answer = await whileBusy(
      () => Core.saveAttachment(widget.id, attachmentId, name),
    );
    if (!mounted) return;
    showMessage(
      context,
      answer.ok
          ? 'Saved to ${answer.body['path']}'
          : answer.error ?? 'Couldn\'t save that.',
    );
  }

  /// Through the core, so the value never passes through Dart; secrets clear after 30 seconds.
  Future<void> copy(String label, String what, {bool secret = true}) async {
    if (asksAgain()) {
      setState(() {});
      return;
    }
    final answer = await widget.core.copy(widget.id, what);
    if (!mounted) return;
    showMessage(
      context,
      answer.ok
          ? (secret
                ? 'Copied the $label. It clears in 30 seconds.'
                : 'Copied the $label')
          : answer.error ?? 'Couldn\'t copy.',
    );
  }

  /// The master password, for an item that asks for it again.
  final masterPassword = TextEditingController();
  String? promptError;
  bool confirming = false;

  Future<void> confirm() async {
    if (masterPassword.text.isEmpty) return;
    setState(() {
      confirming = true;
      promptError = null;
    });
    final answer = await whileBusy(
      () => Core.confirmMasterPassword(widget.id, masterPassword.text),
    );
    if (!mounted) return;
    masterPassword.clear();
    setState(() {
      confirming = false;
      if (answer.ok) {
        item = widget.core.item(widget.id);
      } else {
        promptError = answer.error;
      }
    });
    tick();
  }

  /// What an item that asks for the master password shows until it's given:
  /// its name, and the question.
  Widget askForMasterPassword(BuildContext context, Map<String, dynamic> item) {
    final c = context.colors;
    return ListView(
      padding: widget.phone
          ? const EdgeInsets.fromLTRB(20, 8, 20, 28)
          : const EdgeInsets.fromLTRB(28, 26, 28, 28),
      children: [
        Row(
          children: [
            Avatar(
              name: item['name'] as String,
              kind: item['kind'] as String,
              size: 48,
            ),
            const SizedBox(width: 16),
            Expanded(
              child: Text(
                item['name'] as String,
                style: Theme.of(context).textTheme.titleLarge
                    ?.copyWith(fontWeight: FontWeight.w700),
              ),
            ),
          ],
        ),
        const SizedBox(height: 22),
        Text(
          'This item asks for your master password.',
          style: TextStyle(color: c.muted),
        ),
        const SizedBox(height: 12),
        TextField(
          controller: masterPassword,
          obscureText: true,
          autofocus: true,
          decoration: InputDecoration(
            labelText: 'Master password',
            errorText: promptError,
          ),
          onSubmitted: (_) => confirm(),
        ),
        const SizedBox(height: 12),
        Align(
          alignment: Alignment.centerLeft,
          child: WorkingButton(
            label: 'Continue',
            working: confirming,
            onPressed: confirm,
          ),
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final item = this.item;
    if (item == null) {
      return widget.phone
          ? Scaffold(appBar: AppBar())
          : const SizedBox.shrink();
    }
    final asking = item['reprompt'] == true && item['mayUse'] != true;
    final body = asking
        ? askForMasterPassword(context, item)
        : details(context, item);
    if (!widget.phone) return body;
    return Scaffold(
      appBar: AppBar(
        actions: [
          if (!asking) ...[
            if (item['favorite'] == true)
              const Padding(
                padding: EdgeInsets.symmetric(horizontal: 10),
                child: Glyph(
                  Glyphs.star,
                  size: 22,
                  color: favoriteEdge,
                  fill: favoriteColor,
                  strokeWidth: 1.6,
                  semanticLabel: 'Favorite',
                ),
              ),
            // Some kinds are the web vault's to edit, for now.
            if (item['editable'] != false)
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 4),
                child: OutlinedButton(
                  onPressed: edit,
                  child: const Text('Edit'),
                ),
              ),
            IconButton(
              tooltip: 'Move to trash',
              icon: const Icon(Icons.delete_outline_rounded),
              onPressed: () => widget.onTrash(item['name'] as String),
            ),
          ],
          const SizedBox(width: 8),
        ],
      ),
      body: body,
    );
  }

  /// Its details, each a row on a card with what can be done with it.
  Widget details(BuildContext context, Map<String, dynamic> item) {
    final c = context.colors;
    final scheme = Theme.of(context).colorScheme;
    final phone = widget.phone;
    final valueSize = phone ? 17.0 : 16.0;
    final rows = <Widget>[];
    Widget row(String label, Widget value, {List<Widget> actions = const []}) =>
        ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 68),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 8, 8),
            child: Row(
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(
                        label,
                        style: TextStyle(
                          color: c.muted,
                          fontSize: 13,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      const SizedBox(height: 3),
                      value,
                    ],
                  ),
                ),
                for (final a in actions) ...[const SizedBox(width: 4), a],
              ],
            ),
          ),
        );
    // Big, tinted, and in the brand color: the thing people come here to do.
    Widget action(String tooltip, Widget icon, VoidCallback onPressed) =>
        IconButton(
          tooltip: tooltip,
          style: IconButton.styleFrom(
            backgroundColor: c.tint,
            foregroundColor: scheme.primary,
            minimumSize: const Size(44, 44),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(12),
            ),
          ),
          icon: icon,
          onPressed: onPressed,
        );
    final copyGlyph = Glyph(Glyphs.copy, size: 20, color: scheme.primary);
    Widget copyButton(String label, String what, {bool secret = true}) =>
        action(
          'Copy ${label.toLowerCase()}',
          copyGlyph,
          () => copy(label, what, secret: secret),
        );
    Widget secretRow(String label, String what, {bool canCopy = true}) => row(
      label,
      // Google Sans Flex, like everything else: no system font, so it looks the same everywhere.
      SelectableText(
        revealed[what] ?? '••••••••••••',
        style: TextStyle(fontSize: valueSize, letterSpacing: 0.6),
      ),
      actions: [
        IconButton(
          tooltip: revealed.containsKey(what) ? 'Hide' : 'Reveal',
          icon: Glyph(
            revealed.containsKey(what) ? Glyphs.eyeOff : Glyphs.eye,
            size: 20,
            color: c.muted,
          ),
          onPressed: () => toggle(what),
        ),
        if (canCopy) copyButton(label.toLowerCase(), what),
      ],
    );

    if (item['username'] != null) {
      rows.add(
        row(
          'Username',
          SelectableText(
            item['username'] as String,
            style: TextStyle(fontSize: valueSize),
          ),
          actions: [copyButton('username', 'username', secret: false)],
        ),
      );
    }
    if (item['hasPassword'] == true) {
      rows.add(secretRow('Password', 'password'));
    }
    final code = totp?['code'] as String?;
    if (item['hasTotp'] == true && code != null) {
      final left = (totp!['secondsLeft'] as num).toInt();
      final period = (totp!['period'] as num).toInt();
      rows.add(
        row(
          'One-time code',
          Text(
            '${code.substring(0, code.length ~/ 2)} ${code.substring(code.length ~/ 2)}',
            style: TextStyle(
              fontSize: valueSize + 2,
              fontWeight: FontWeight.w600,
              letterSpacing: 1.5,
            ),
          ),
          actions: [
            // The seconds left, inside a ring that runs down with them.
            Semantics(
              label: '$left seconds left',
              excludeSemantics: true,
              child: SizedBox(
                width: 30,
                height: 30,
                child: Stack(
                  alignment: Alignment.center,
                  children: [
                    Positioned.fill(
                      child: CircularProgressIndicator(
                        value: left / period,
                        strokeWidth: 3,
                        backgroundColor: c.line,
                      ),
                    ),
                    Text(
                      '$left',
                      textScaler: TextScaler.noScaling,
                      style: TextStyle(
                        color: scheme.primary,
                        fontSize: 11,
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                  ],
                ),
              ),
            ),
            const SizedBox(width: 4),
            copyButton('one-time code', 'totp'),
          ],
        ),
      );
    }
    for (final uri in (item['uris'] as List).cast<String>()) {
      rows.add(
        row(
          'Website',
          Text(
            uri,
            style: TextStyle(color: scheme.primary, fontSize: valueSize),
          ),
          actions: [
            action('Copy website', copyGlyph, () {
              Clipboard.setData(ClipboardData(text: uri));
              showMessage(context, 'Copied the website');
            }),
          ],
        ),
      );
    }
    // A card's or an identity's details: plain ones copy as they are,
    // secrets reveal and copy like a password.
    void plainRow(String label, String? value, String what) {
      if (value == null || value.isEmpty) return;
      rows.add(
        row(
          label,
          SelectableText(value, style: TextStyle(fontSize: valueSize)),
          actions: [copyButton(label.toLowerCase(), what, secret: false)],
        ),
      );
    }

    final cardDetails = item['card'] as Map<String, dynamic>?;
    if (cardDetails != null) {
      plainRow(
        'Cardholder name',
        cardDetails['cardholderName'] as String?,
        'cardholderName',
      );
      final brand = cardDetails['brand'] as String?;
      if (brand != null && brand.isNotEmpty) {
        rows.add(
          row('Brand', Text(brand, style: TextStyle(fontSize: valueSize))),
        );
      }
      if (cardDetails['hasNumber'] == true) {
        rows.add(secretRow('Number', 'cardNumber'));
      }
      plainRow(
        'Expiration',
        cardDetails['expiration'] as String?,
        'expiration',
      );
      if (cardDetails['hasCode'] == true) {
        rows.add(secretRow('Security code', 'cardCode'));
      }
    }
    final sshKey = item['sshKey'] as Map<String, dynamic>?;
    if (sshKey != null) {
      plainRow('Public key', sshKey['publicKey'] as String?, 'sshPublicKey');
      plainRow(
        'Fingerprint',
        sshKey['fingerprint'] as String?,
        'sshFingerprint',
      );
      if (sshKey['hasPrivateKey'] == true) {
        rows.add(secretRow('Private key', 'sshPrivateKey'));
      }
    }
    final identityDetails = item['identity'] as Map<String, dynamic>?;
    if (identityDetails != null) {
      plainRow('Name', identityDetails['fullName'] as String?, 'fullName');
      plainRow(
        'Username',
        identityDetails['username'] as String?,
        'identityUsername',
      );
      plainRow('Company', identityDetails['company'] as String?, 'company');
      plainRow('Email', identityDetails['email'] as String?, 'email');
      plainRow('Phone', identityDetails['phone'] as String?, 'phone');
      plainRow('Address', identityDetails['address'] as String?, 'address');
      if (identityDetails['hasSsn'] == true) {
        rows.add(secretRow('Social Security number', 'ssn'));
      }
      if (identityDetails['hasPassportNumber'] == true) {
        rows.add(secretRow('Passport number', 'passportNumber'));
      }
      if (identityDetails['hasLicenseNumber'] == true) {
        rows.add(secretRow('License number', 'licenseNumber'));
      }
    }
    final fields = (item['fields'] as List).cast<Map<String, dynamic>>();
    for (var i = 0; i < fields.length; i++) {
      final f = fields[i];
      // Our own data (fill rules, an expiration date) isn't a custom field.
      if (f['reserved'] == true) continue;
      rows.add(
        f['hidden'] == true
            ? secretRow(f['name'] as String, 'field:$i')
            : row(
                f['name'] as String,
                SelectableText(
                  (f['value'] ?? '') as String,
                  style: TextStyle(fontSize: valueSize),
                ),
                actions: [
                  copyButton(
                    (f['name'] as String).toLowerCase(),
                    'field:$i',
                    secret: false,
                  ),
                ],
              ),
      );
    }
    // Attachments: saved to Downloads, or to Files on an iPhone.
    for (final a
        in ((item['attachments'] as List?) ?? const [])
            .cast<Map<String, dynamic>>()) {
      rows.add(
        row(
          'Attachment',
          Text(
            '${a['name']}${a['size'] == null ? '' : ' · ${fileSize(a['size'] as num)}'}',
            style: TextStyle(fontSize: valueSize),
          ),
          // One whose name or key doesn't open is listed, but not saved.
          actions: [
            if (a['opens'] != false)
              action(
                'Save ${a['name']}',
                Icon(Icons.download_rounded, size: 20, color: scheme.primary),
                () => saveAttachment(a['id'] as String, a['name'] as String),
              ),
          ],
        ),
      );
    }
    // A login's passkey: who it's for and when it was made, never its key.
    final passkey = item['passkey'] as Map<String, dynamic>?;
    if (passkey != null) {
      final site = passkey['rpName'] as String? ?? passkey['rpId'] as String;
      final created = madeOn(passkey['created'] as String?);
      rows.add(
        row(
          'Passkey',
          Text(
            created == null ? 'For $site' : 'For $site · created $created',
            style: TextStyle(fontSize: valueSize),
          ),
        ),
      );
    }
    final expires = item['expires'] as String?;
    if (expires != null && expires.isNotEmpty) {
      rows.add(
        row('Expires', Text(expires, style: TextStyle(fontSize: valueSize))),
      );
    }
    if (item['hasNotes'] == true) {
      rows.add(secretRow('Notes', 'notes', canCopy: false));
    }

    final kind = ItemKind.of(item['kind'] as String);
    final where = [
      item['organization'],
      item['folder'],
    ].whereType<String>().join(phone ? ' › ' : ' · ');
    final card = RestingCard(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (var i = 0; i < rows.length; i++) ...[
            if (i > 0) const Divider(indent: 16),
            rows[i],
          ],
        ],
      ),
    );
    if (phone) {
      return ListView(
        padding: const EdgeInsets.fromLTRB(16, 4, 16, 32),
        children: [
          // A big tile, its name, and what it is and where.
          Center(
            child: DecoratedBox(
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(20),
                boxShadow: c.lifted,
              ),
              child: Avatar(
                name: item['name'] as String,
                kind: item['kind'] as String,
                size: 72,
              ),
            ),
          ),
          const SizedBox(height: 14),
          Text(
            item['name'] as String,
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 24, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 6),
          Text.rich(
            TextSpan(
              children: [
                WidgetSpan(
                  alignment: PlaceholderAlignment.middle,
                  child: Padding(
                    padding: const EdgeInsets.only(right: 6),
                    child: JewelTile(jewel: kind, size: 18),
                  ),
                ),
                TextSpan(
                  text: '${kind.one} in ${where.isEmpty ? 'My vault' : where}',
                ),
              ],
            ),
            textAlign: TextAlign.center,
            style: TextStyle(color: c.muted, fontSize: 14),
          ),
          const SizedBox(height: 22),
          if (rows.isNotEmpty) card,
        ],
      );
    }
    // On a computer: a raised card, as the web vault has it, with the
    // details in a well inside.
    return ListView(
      padding: const EdgeInsets.fromLTRB(12, 12, 20, 20),
      children: [
        Align(
          alignment: Alignment.topLeft,
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 600),
            child: Container(
              padding: const EdgeInsets.all(24),
              decoration: BoxDecoration(
                color: c.pane,
                borderRadius: BorderRadius.circular(16),
                boxShadow: c.lifted,
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      DecoratedBox(
                        decoration: BoxDecoration(
                          borderRadius: BorderRadius.circular(15),
                          boxShadow: c.lifted,
                        ),
                        child: Avatar(
                          name: item['name'] as String,
                          kind: item['kind'] as String,
                          size: 56,
                        ),
                      ),
                      const SizedBox(width: 14),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(
                              item['name'] as String,
                              style: const TextStyle(
                                fontSize: 22,
                                fontWeight: FontWeight.w700,
                              ),
                            ),
                            const SizedBox(height: 2),
                            Text(
                              '${kind.one} · ${where.isEmpty ? 'My vault' : where}',
                              style: TextStyle(color: c.muted, fontSize: 13),
                            ),
                          ],
                        ),
                      ),
                      if (item['favorite'] == true)
                        const Padding(
                          padding: EdgeInsets.symmetric(horizontal: 8),
                          child: Glyph(
                            Glyphs.star,
                            size: 20,
                            color: favoriteEdge,
                            fill: favoriteColor,
                            strokeWidth: 1.6,
                            semanticLabel: 'Favorite',
                          ),
                        ),
                      // Some kinds are the web vault's to edit, for now.
                      if (item['editable'] != false)
                        Padding(
                          padding: const EdgeInsets.only(left: 4),
                          child: OutlinedButton(
                            onPressed: edit,
                            child: const Text('Edit'),
                          ),
                        ),
                      IconButton(
                        tooltip: 'Move to trash',
                        icon: const Icon(Icons.delete_outline_rounded),
                        onPressed: () => widget.onTrash(item['name'] as String),
                      ),
                    ],
                  ),
                  if (rows.isNotEmpty) ...[
                    const SizedBox(height: 18),
                    DecoratedBox(
                      decoration: BoxDecoration(
                        color: c.well,
                        borderRadius: BorderRadius.circular(12),
                        border: Border.all(color: c.line),
                      ),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          for (var i = 0; i < rows.length; i++) ...[
                            if (i > 0) Divider(indent: 16, color: c.line),
                            rows[i],
                          ],
                        ],
                      ),
                    ),
                  ],
                ],
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// Asks for the master password for an item that asks for it again
/// (docs/core.md, "Asking for the master password again"); true once it's
/// given.
Future<bool> askForMasterPassword(
  BuildContext context,
  String id,
  String name,
) async =>
    await showDialog<bool>(
      context: context,
      builder: (_) => _MasterPasswordDialog(id: id, name: name),
    ) ==
    true;

class _MasterPasswordDialog extends StatefulWidget {
  const _MasterPasswordDialog({required this.id, required this.name});

  final String id;
  final String name;

  @override
  State<_MasterPasswordDialog> createState() => _MasterPasswordDialogState();
}

class _MasterPasswordDialogState extends State<_MasterPasswordDialog> {
  final password = TextEditingController();
  String? error;
  bool working = false;

  @override
  void dispose() {
    password.dispose();
    super.dispose();
  }

  Future<void> confirm() async {
    if (password.text.isEmpty || working) return;
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(
      () => Core.confirmMasterPassword(widget.id, password.text),
    );
    if (!mounted) return;
    password.clear();
    if (answer.ok) {
      Navigator.of(context).pop(true);
      return;
    }
    setState(() {
      working = false;
      error = answer.error;
    });
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(widget.name),
    content: SizedBox(
      width: 360,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('This item asks for your master password.'),
          const SizedBox(height: 12),
          TextField(
            controller: password,
            obscureText: true,
            autofocus: true,
            decoration: InputDecoration(
              labelText: 'Master password',
              errorText: error,
            ),
            onSubmitted: (_) => confirm(),
          ),
        ],
      ),
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.of(context).pop(false),
        child: const Text('Cancel'),
      ),
      WorkingButton(label: 'Continue', working: working, onPressed: confirm),
    ],
  );
}

/// A file's size for people: "37 bytes", "1.2 KB", "3.4 MB".
String fileSize(num bytes) {
  if (bytes < 1000) return '${bytes.toInt()} bytes';
  if (bytes < 1000 * 1000) return '${(bytes / 1000).toStringAsFixed(1)} KB';
  return '${(bytes / (1000 * 1000)).toStringAsFixed(1)} MB';
}

/// A folder's Rename and Delete, beside its name in the sidebar, or (large)
/// in a phone's header while the folder is shown.
class FolderMenu extends StatelessWidget {
  const FolderMenu({
    super.key,
    required this.name,
    required this.color,
    required this.onRename,
    required this.onDelete,
    this.large = false,
  });

  final String name;
  final Color color;
  final VoidCallback onRename;
  final VoidCallback onDelete;
  final bool large;

  @override
  Widget build(BuildContext context) {
    final menu = PopupMenuButton<VoidCallback>(
      tooltip: 'More for $name',
      padding: large ? const EdgeInsets.all(8) : EdgeInsets.zero,
      iconSize: large ? 24 : 16,
      icon: Icon(
        large ? Icons.more_vert_rounded : Icons.more_horiz_rounded,
        color: color,
      ),
      onSelected: (action) => action(),
      itemBuilder: (context) => [
        PopupMenuItem(value: onRename, child: const Text('Rename')),
        PopupMenuItem(value: onDelete, child: const Text('Delete')),
      ],
    );
    return large ? menu : SizedBox(width: 24, height: 20, child: menu);
  }
}

/// A folder's name, asked for: null when cancelled.
Future<String?> askFolderName(
  BuildContext context,
  String title, [
  String name = '',
]) => showDialog<String>(
  context: context,
  builder: (context) => _FolderNameDialog(title: title, name: name),
);

class _FolderNameDialog extends StatefulWidget {
  const _FolderNameDialog({required this.title, required this.name});

  final String title;
  final String name;

  @override
  State<_FolderNameDialog> createState() => _FolderNameDialogState();
}

class _FolderNameDialogState extends State<_FolderNameDialog> {
  late final field = TextEditingController(text: widget.name);

  @override
  void dispose() {
    field.dispose();
    super.dispose();
  }

  void done() {
    final name = field.text.trim();
    if (name.isNotEmpty) Navigator.pop(context, name);
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(widget.title),
    content: TextField(
      controller: field,
      autofocus: true,
      decoration: const InputDecoration(labelText: 'Name'),
      onChanged: (_) => setState(() {}),
      onSubmitted: (_) => done(),
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.pop(context),
        child: const Text('Cancel'),
      ),
      FilledButton(
        onPressed: field.text.trim().isEmpty ? null : done,
        child: const Text('Save'),
      ),
    ],
  );
}

/// A date the server gave (ISO 8601) as people read it: "October 20, 2026".
String? madeOn(String? iso) {
  final at = iso == null ? null : DateTime.tryParse(iso)?.toLocal();
  if (at == null) return null;
  const months = [
    'January',
    'February',
    'March',
    'April',
    'May',
    'June',
    'July',
    'August',
    'September',
    'October',
    'November',
    'December',
  ];
  return '${months[at.month - 1]} ${at.day}, ${at.year}';
}
