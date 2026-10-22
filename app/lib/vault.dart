// The vault: a sidebar (all items, favorites, folders, organizations), the
// list with search, and the item, side by side on a wide window and one at a
// time on a narrow one. Copying goes through the core, so a password never
// passes through Dart; secrets reach the screen only while revealed.
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'editor.dart';
import 'settings.dart';
import 'theme.dart';
import 'widgets.dart';

enum Filter { all, favorites, folder, organization }

class VaultScreen extends StatefulWidget {
  const VaultScreen({
    super.key,
    required this.core,
    required this.email,
    required this.mode,
    required this.onMode,
    required this.onLock,
    required this.onSignInAgain,
    this.syncOnOpen = true,
  });

  final Core core;
  final String email;
  final ThemeMode mode;
  final ValueChanged<ThemeMode> onMode;
  final VoidCallback onLock;

  /// Signed out on the server: sign in again to get changes.
  final VoidCallback onSignInAgain;
  final bool syncOnOpen;

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
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (widget.syncOnOpen) sync();
      saveOffered();
    });
  }

  @override
  void dispose() {
    lifecycle.dispose();
    super.dispose();
  }

  /// A sign-in Android's autofill offered to save: the editor, filled in.
  Future<void> saveOffered() async {
    if (!mounted) return;
    final save = await Core.pendingSave();
    if (save == null || !mounted) return;
    // A new password for a login it has comes with that login's id.
    await edit(save['id'] as String?, save);
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
  };

  Future<void> edit([String? id, Map<String, dynamic>? prefill]) async {
    final wide = MediaQuery.sizeOf(context).width >= 900;
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

  Future<void> trash(String id, String name) async {
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
    if (sure != true) return;
    final answer = await whileBusy(() => Core.trash(id));
    if (!mounted) return;
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t move it to the trash.');
      return;
    }
    setState(() => selected = null);
    reload();
  }

  @override
  Widget build(BuildContext context) {
    final list = shown;
    final width = MediaQuery.sizeOf(context).width;
    final wide = width >= 900;
    final pane = selected == null
        ? null
        : ItemPane(
            key: ValueKey(selected),
            core: widget.core,
            id: selected!,
            onEdit: () => edit(selected),
            onTrash: (name) => trash(selected!, name),
          );

    final listPane = ItemList(
      header: wide
          ? null
          : PhoneHeader(
              mode: widget.mode,
              onMode: widget.onMode,
              onLock: widget.onLock,
            ),
      items: list,
      selected: selected,
      syncing: syncing,
      onSearch: (q) {
        query = q;
        reload();
      },
      onNew: () => edit(),
      onSync: sync,
      onSelect: (id) {
        if (wide) {
          setState(() => selected = id);
        } else {
          Navigator.of(context).push(
            MaterialPageRoute(
              builder: (_) => Scaffold(
                appBar: AppBar(),
                body: ItemPane(
                  core: widget.core,
                  id: id,
                  onEdit: () => edit(id),
                  onTrash: (name) async {
                    Navigator.of(context).pop();
                    await trash(id, name);
                  },
                ),
              ),
            ),
          );
        }
      },
    );

    return Scaffold(
      body: Column(
        children: [
          if (signedOut)
            MaterialBanner(
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
            ),
          Expanded(
            child: Row(
              children: [
                if (wide)
                  Sidebar(
                    email: widget.email,
                    filter: filter,
                    filterId: filterId,
                    places: places,
                    counts: (
                      all.length,
                      all.where((i) => i['favorite'] == true).length,
                    ),
                    onChoose: (f, [id]) => setState(() {
                      filter = f;
                      filterId = id;
                    }),
                    mode: widget.mode,
                    onMode: widget.onMode,
                    onLock: widget.onLock,
                  ),
                SizedBox(width: wide ? 360 : width, child: listPane),
                if (wide) const VerticalDivider(width: 1),
                if (wide)
                  Expanded(
                    child: Container(
                      color: context.colors.pane,
                      child:
                          pane ??
                          Center(
                            child: Text(
                              'Choose an item',
                              style: TextStyle(color: context.colors.muted),
                            ),
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
}

void showMessage(BuildContext context, String text) =>
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(text), duration: const Duration(seconds: 3)),
    );

class Sidebar extends StatelessWidget {
  const Sidebar({
    super.key,
    required this.email,
    required this.filter,
    required this.filterId,
    required this.places,
    required this.counts,
    required this.onChoose,
    required this.mode,
    required this.onMode,
    required this.onLock,
  });

  final String email;
  final Filter filter;
  final String? filterId;
  final Map<String, dynamic> places;
  final (int, int) counts;
  final void Function(Filter, [String?]) onChoose;
  final ThemeMode mode;
  final ValueChanged<ThemeMode> onMode;
  final VoidCallback onLock;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget entry(
      IconData icon,
      String label,
      bool on,
      VoidCallback tap, [
      int? count,
    ]) => Padding(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 1),
      child: Material(
        color: on ? Colors.white.withValues(alpha: 0.12) : Colors.transparent,
        borderRadius: BorderRadius.circular(8),
        child: InkWell(
          borderRadius: BorderRadius.circular(8),
          onTap: tap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 9),
            child: Row(
              children: [
                Icon(icon, size: 18, color: c.sidebarText),
                const SizedBox(width: 10),
                Expanded(
                  child: Text(
                    label,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: c.sidebarText,
                      fontWeight: on ? FontWeight.w600 : FontWeight.w400,
                    ),
                  ),
                ),
                if (count != null)
                  Text(
                    '$count',
                    style: TextStyle(
                      color: c.sidebarText.withValues(alpha: 0.7),
                      fontSize: 12,
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
    Widget headingText(String text) => Padding(
      padding: const EdgeInsets.fromLTRB(22, 18, 16, 6),
      child: Text(
        text.toUpperCase(),
        style: TextStyle(
          color: c.sidebarText.withValues(alpha: 0.6),
          fontSize: 11,
          letterSpacing: 0.8,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
    final folders = (places['folders'] as List).cast<Map<String, dynamic>>();
    final organizations = (places['organizations'] as List)
        .cast<Map<String, dynamic>>();
    return Container(
      width: 232,
      color: c.sidebar,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 22, 16, 12),
            child: Row(
              children: [
                Mark(size: 24, color: c.sidebarText),
                const SizedBox(width: 10),
                Text(
                  'PassBundle',
                  style: TextStyle(
                    color: c.sidebarText,
                    fontWeight: FontWeight.w700,
                    fontSize: 17,
                  ),
                ),
              ],
            ),
          ),
          Expanded(
            child: ListView(
              children: [
                entry(
                  Icons.grid_view_rounded,
                  'All items',
                  filter == Filter.all,
                  () => onChoose(Filter.all),
                  counts.$1,
                ),
                entry(
                  Icons.star_outline_rounded,
                  'Favorites',
                  filter == Filter.favorites,
                  () => onChoose(Filter.favorites),
                  counts.$2,
                ),
                if (folders.isNotEmpty) headingText('Folders'),
                for (final f in folders)
                  entry(
                    Icons.folder_outlined,
                    f['name'] as String,
                    filter == Filter.folder && filterId == f['id'],
                    () => onChoose(Filter.folder, f['id'] as String),
                  ),
                if (organizations.isNotEmpty) headingText('Organizations'),
                for (final o in organizations)
                  entry(
                    Icons.business_outlined,
                    o['name'] as String,
                    filter == Filter.organization && filterId == o['id'],
                    () => onChoose(Filter.organization, o['id'] as String),
                  ),
              ],
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 4, 16, 0),
            child: Text(
              email,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: c.sidebarText.withValues(alpha: 0.7),
                fontSize: 12,
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.all(10),
            child: Row(
              children: [
                ThemeMenu(mode: mode, onMode: onMode, color: c.sidebarText),
                SettingsButton(color: c.sidebarText),
                const Spacer(),
                TextButton.icon(
                  onPressed: onLock,
                  icon: Icon(
                    Icons.lock_outline_rounded,
                    size: 18,
                    color: c.sidebarText,
                  ),
                  label: Text('Lock', style: TextStyle(color: c.sidebarText)),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The top of the list on a phone-width window, where there's no sidebar.
class PhoneHeader extends StatelessWidget {
  const PhoneHeader({
    super.key,
    required this.mode,
    required this.onMode,
    required this.onLock,
  });

  final ThemeMode mode;
  final ValueChanged<ThemeMode> onMode;
  final VoidCallback onLock;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(20, 16, 8, 0),
    child: Row(
      children: [
        const Mark(size: 22),
        const SizedBox(width: 10),
        const Expanded(
          child: Text(
            'PassBundle',
            style: TextStyle(fontWeight: FontWeight.w700, fontSize: 17),
          ),
        ),
        ThemeMenu(mode: mode, onMode: onMode),
        const SettingsButton(),
        IconButton(
          tooltip: 'Lock',
          icon: const Icon(Icons.lock_outline_rounded),
          onPressed: onLock,
        ),
      ],
    ),
  );
}

class ItemList extends StatelessWidget {
  const ItemList({
    super.key,
    this.header,
    required this.items,
    required this.selected,
    required this.syncing,
    required this.onSearch,
    required this.onNew,
    required this.onSync,
    required this.onSelect,
  });

  final Widget? header;
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
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (header != null) SafeArea(bottom: false, child: header!),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 18, 8, 12),
          child: Row(
            children: [
              Expanded(
                child: TextField(
                  onChanged: onSearch,
                  decoration: const InputDecoration(
                    hintText: 'Search',
                    prefixIcon: Icon(Icons.search_rounded, size: 20),
                  ),
                ),
              ),
              const SizedBox(width: 4),
              syncing
                  ? const Padding(
                      padding: EdgeInsets.all(12),
                      child: SizedBox(
                        width: 18,
                        height: 18,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      ),
                    )
                  : IconButton(
                      tooltip: 'Sync',
                      icon: const Icon(Icons.sync_rounded),
                      onPressed: onSync,
                    ),
              IconButton.filled(
                tooltip: 'New item',
                icon: const Icon(Icons.add_rounded),
                onPressed: onNew,
              ),
            ],
          ),
        ),
        const Divider(),
        Expanded(
          child: items.isEmpty
              ? Center(
                  child: Text('Nothing here', style: TextStyle(color: c.muted)),
                )
              : ListView.builder(
                  itemCount: items.length,
                  itemBuilder: (context, i) {
                    final item = items[i];
                    final on = item['id'] == selected;
                    return Material(
                      color: on ? c.selected : Colors.transparent,
                      child: InkWell(
                        onTap: () => onSelect(item['id'] as String),
                        child: Padding(
                          padding: const EdgeInsets.symmetric(
                            horizontal: 16,
                            vertical: 11,
                          ),
                          child: Row(
                            children: [
                              Avatar(
                                name: item['name'] as String,
                                kind: item['kind'] as String,
                              ),
                              const SizedBox(width: 12),
                              Expanded(
                                child: Column(
                                  crossAxisAlignment: CrossAxisAlignment.start,
                                  children: [
                                    Text(
                                      item['name'] as String,
                                      maxLines: 1,
                                      overflow: TextOverflow.ellipsis,
                                      style: const TextStyle(
                                        fontWeight: FontWeight.w600,
                                      ),
                                    ),
                                    if ((item['subtitle'] as String).isNotEmpty)
                                      Text(
                                        item['subtitle'] as String,
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
                                Icon(
                                  Icons.star_rounded,
                                  size: 16,
                                  color: c.muted,
                                ),
                            ],
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

class ItemPane extends StatefulWidget {
  const ItemPane({
    super.key,
    required this.core,
    required this.id,
    required this.onEdit,
    required this.onTrash,
  });

  final Core core;
  final String id;
  final VoidCallback onEdit;
  final ValueChanged<String> onTrash;

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
    super.dispose();
  }

  void toggle(String what) => setState(() {
    if (revealed.remove(what) == null) {
      final secret = widget.core.reveal(widget.id, what);
      if (secret != null) revealed[what] = secret;
    }
  });

  /// Through the core, so the value never passes through Dart; secrets clear after 30 seconds.
  Future<void> copy(String label, String what, {bool secret = true}) async {
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

  @override
  Widget build(BuildContext context) {
    final item = this.item;
    if (item == null) return const SizedBox.shrink();
    final c = context.colors;
    final rows = <Widget>[];
    Widget row(String label, Widget value, {List<Widget> actions = const []}) =>
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 12),
          child: Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      label,
                      style: TextStyle(
                        color: c.muted,
                        fontSize: 12,
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                    const SizedBox(height: 3),
                    value,
                  ],
                ),
              ),
              ...actions,
            ],
          ),
        );
    IconButton copyButton(String label, String what, {bool secret = true}) =>
        IconButton(
          tooltip: 'Copy ${label.toLowerCase()}',
          icon: const Icon(Icons.copy_rounded, size: 19),
          onPressed: () => copy(label, what, secret: secret),
        );
    Widget secretRow(String label, String what, {bool canCopy = true}) => row(
      label,
      // Inter, like everything else: no system font, so it looks the same everywhere.
      SelectableText(
        revealed[what] ?? '••••••••••••',
        style: const TextStyle(fontSize: 15, letterSpacing: 0.4),
      ),
      actions: [
        IconButton(
          tooltip: revealed.containsKey(what) ? 'Hide' : 'Reveal',
          icon: Icon(
            revealed.containsKey(what)
                ? Icons.visibility_off_outlined
                : Icons.visibility_outlined,
            size: 20,
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
            style: const TextStyle(fontSize: 15),
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
            style: const TextStyle(
              fontSize: 18,
              fontWeight: FontWeight.w600,
              letterSpacing: 1.5,
            ),
          ),
          actions: [
            SizedBox(
              width: 22,
              height: 22,
              child: CircularProgressIndicator(
                value: left / period,
                strokeWidth: 2.5,
                backgroundColor: c.line,
              ),
            ),
            const SizedBox(width: 6),
            Text('${left}s', style: TextStyle(color: c.muted, fontSize: 12)),
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
            style: TextStyle(
              color: Theme.of(context).colorScheme.primary,
              fontSize: 15,
            ),
          ),
          actions: [
            IconButton(
              tooltip: 'Copy website',
              icon: const Icon(Icons.copy_rounded, size: 19),
              onPressed: () {
                Clipboard.setData(ClipboardData(text: uri));
                showMessage(context, 'Copied the website');
              },
            ),
          ],
        ),
      );
    }
    final fields = (item['fields'] as List).cast<Map<String, dynamic>>();
    for (var i = 0; i < fields.length; i++) {
      final f = fields[i];
      rows.add(
        f['hidden'] == true
            ? secretRow(f['name'] as String, 'field:$i')
            : row(
                f['name'] as String,
                SelectableText(
                  (f['value'] ?? '') as String,
                  style: const TextStyle(fontSize: 15),
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
    if (item['hasNotes'] == true) {
      rows.add(secretRow('Notes', 'notes', canCopy: false));
    }

    Widget card(List<Widget> children) => Container(
      decoration: BoxDecoration(
        color: Theme.of(context).colorScheme.surface,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: c.line),
      ),
      child: Column(
        children: [
          for (var i = 0; i < children.length; i++) ...[
            if (i > 0) const Divider(indent: 18, endIndent: 18),
            children[i],
          ],
        ],
      ),
    );

    final where = [
      item['organization'],
      item['folder'],
    ].whereType<String>().join(' · ');
    return ListView(
      padding: const EdgeInsets.fromLTRB(28, 26, 28, 28),
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
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    item['name'] as String,
                    style: Theme.of(context).textTheme.titleLarge
                        ?.copyWith(fontWeight: FontWeight.w700),
                  ),
                  Text(
                    where.isEmpty ? 'My vault' : where,
                    style: TextStyle(color: c.muted, fontSize: 13),
                  ),
                ],
              ),
            ),
            if (item['favorite'] == true)
              Icon(
                Icons.star_rounded,
                color: Theme.of(context).colorScheme.primary,
              ),
            // Cards, identities, and SSH keys are the web vault's to edit, for now.
            if (item['editable'] != false)
              IconButton(
                tooltip: 'Edit',
                icon: const Icon(Icons.edit_outlined),
                onPressed: widget.onEdit,
              ),
            IconButton(
              tooltip: 'Move to trash',
              icon: const Icon(Icons.delete_outline_rounded),
              onPressed: () => widget.onTrash(item['name'] as String),
            ),
          ],
        ),
        const SizedBox(height: 22),
        if (rows.isNotEmpty) card(rows),
      ],
    );
  }
}
