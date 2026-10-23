// Adding and editing an item. Editing shows its secrets in the fields, so
// they pass through Dart while the editor is open, as typing them does; the
// core encrypts what's saved (docs/core.md, "Memory"). A new item comes from
// the New item picker (new_item.dart): a kind, or a template with its
// fields. Where it goes is one Folder picker, grouped by owner, as the web
// vault has it: Personal and its folders, then each organization's shared
// folders the person may add items to.
import 'package:flutter/material.dart';

import 'core.dart';
import 'generator.dart';
import 'glyphs.dart';
import 'new_item.dart';
import 'places.dart';
import 'theme.dart';
import 'widgets.dart';

/// Opens the editor; resolves to the saved item's id ('' for a new one), or
/// null if cancelled. A new item is the kind or template `choice` picked,
/// going to `place` (as [_ItemEditorState.place] says it) unless the person
/// picks another.
Future<String?> openEditor(
  BuildContext context, {
  required Core core,
  required Places places,
  String? id,
  Map<String, dynamic>? prefill,
  NewItemChoice? choice,
  String? place,
  required bool asDialog,
}) {
  final editor = ItemEditor(
    core: core,
    places: places,
    id: id,
    prefill: prefill,
    choice: choice,
    place: place,
  );
  if (asDialog) {
    return showDialog<String>(
      context: context,
      builder: (context) => Dialog(
        clipBehavior: Clip.antiAlias,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 560, maxHeight: 760),
          child: editor,
        ),
      ),
    );
  }
  return Navigator.of(context).push<String>(
    MaterialPageRoute(fullscreenDialog: true, builder: (_) => editor),
  );
}

/// A custom field as the editor has it: upstream's type (0 text, 1 hidden,
/// 2 yes or no), its name, and its value ("true" or "false" for yes or no).
class _Field {
  _Field(String name, String value, this.kind)
    : name = TextEditingController(text: name),
      value = TextEditingController(text: value),
      yes = value == 'true';

  final TextEditingController name;
  final TextEditingController value;
  int kind;
  bool yes;

  bool get hidden => kind == 1;
  bool get boolean => kind == 2;
}

class ItemEditor extends StatefulWidget {
  const ItemEditor({
    super.key,
    required this.core,
    required this.places,
    this.id,
    this.prefill,
    this.choice,
    this.place,
  });

  final Core core;
  final Places places;
  final String? id;

  /// For a new item: the kind or template picked.
  final NewItemChoice? choice;

  /// For a new item: where it goes, unless the person picks another.
  final String? place;

  /// A new login's name, username, password, and website, already filled in
  /// (a sign-in Android's autofill offered to save); for an existing login,
  /// its new password.
  final Map<String, dynamic>? prefill;

  @override
  State<ItemEditor> createState() => _ItemEditorState();
}

class _ItemEditorState extends State<ItemEditor> {
  final name = TextEditingController();
  final username = TextEditingController();
  final password = TextEditingController();
  final totp = TextEditingController();
  final notes = TextEditingController();
  final uris = <TextEditingController>[];
  final fields = <_Field>[];

  /// login, note, card, or identity.
  String kind = 'login';
  bool get note => kind == 'note';

  // A card's details, by upstream's names; the number and code are secrets.
  final card = {
    for (final k in ['cardholderName', 'number', 'expYear', 'code'])
      k: TextEditingController(),
  };
  String? brand;
  String? expMonth;

  // An identity's details, by upstream's names.
  final identity = {
    for (final k in _identityParts.keys) k: TextEditingController(),
  };
  bool favorite = false;
  num? grant;
  bool hidden = true;
  String? folderId;
  String? organizationId;
  String? collectionId;
  String? error;
  bool working = false;

  /// The template a new item is made from: `{id, name, ...}`.
  Map<String, dynamic>? template;

  /// The template an item being edited came from, to name it.
  String? templateName;

  /// An organization's item being edited: the shared folders it's in, the
  /// first of which the Folder picker shows.
  List<String> held = const [];

  /// What the person may do with the item being edited (its shared folders').
  Map<String, dynamic> permissions = const {};

  bool get editing => widget.id != null;

  @override
  void initState() {
    super.initState();
    final id = widget.id;
    final item = id == null ? null : widget.core.item(id);
    // For an item that asks for the master password: the giving it was
    // opened under, which saving hands back (core/src/ffi.rs, pb_save).
    grant = item?['grant'] as num?;
    if (id != null && item != null) {
      final reveal = widget.core.reveal;
      kind = item['kind'] as String;
      templateName = (item['template'] as Map?)?['name'] as String?;
      permissions = Map<String, dynamic>.from(
        (item['permissions'] as Map?) ?? const {},
      );
      final cardDetails = item['card'] as Map<String, dynamic>?;
      if (cardDetails != null) {
        card['cardholderName']!.text =
            (cardDetails['cardholderName'] ?? '') as String;
        card['expYear']!.text = (cardDetails['expYear'] ?? '') as String;
        card['number']!.text = reveal(id, 'cardNumber') ?? '';
        card['code']!.text = reveal(id, 'cardCode') ?? '';
        brand = cardDetails['brand'] as String?;
        expMonth = cardDetails['expMonth'] as String?;
      }
      final identityDetails = item['identity'] as Map<String, dynamic>?;
      if (identityDetails != null) {
        for (final part in _identityParts.keys) {
          identity[part]!.text = _identitySecrets.contains(part)
              ? reveal(id, part) ?? ''
              : (identityDetails[part] ?? '') as String;
        }
      }
      name.text = item['name'] as String;
      username.text = (item['username'] ?? '') as String;
      password.text = reveal(id, 'password') ?? '';
      totp.text = reveal(id, 'totp') ?? '';
      notes.text = reveal(id, 'notes') ?? '';
      uris.addAll(
        (item['uris'] as List).cast<String>().map(
          (u) => TextEditingController(text: u),
        ),
      );
      final itemFields = (item['fields'] as List).cast<Map<String, dynamic>>();
      for (var i = 0; i < itemFields.length; i++) {
        final f = itemFields[i];
        // Linked and nameless fields are the web vault's to edit, and our
        // own data (fill rules, an expiration date, the template) isn't a
        // custom field; saving keeps them (core/src/vault.rs, Kept).
        if (f['editable'] == false) continue;
        final hidden = f['hidden'] == true;
        fields.add(
          _Field(
            f['name'] as String,
            (hidden ? reveal(id, 'field:$i') : f['value'] as String?) ?? '',
            (f['type'] as int?) ?? (hidden ? 1 : 0),
          ),
        );
      }
      favorite = item['favorite'] == true;
      folderId = item['folderId'] as String?;
      organizationId = item['organizationId'] as String?;
      held = (item['collectionIds'] as List?)?.cast<String>() ?? const [];
      // The first of its shared folders the person has.
      collectionId =
          held.where((c) => widget.places.collection(c) != null).firstOrNull ??
          held.firstOrNull;
    }
    // A new item: the kind or template picked, where it's going.
    final choice = widget.choice;
    if (id == null && choice != null) {
      kind = choice.kind;
      template = choice.template;
      for (final f
          in ((template?['fields'] as List?) ?? const [])
              .cast<Map<String, dynamic>>()) {
        final kind = switch (f['kind']) {
          'hidden' => 1,
          'boolean' => 2,
          _ => 0,
        };
        fields.add(
          _Field(
            f['label'] as String,
            (f['value'] as String?) ?? (kind == 2 ? 'false' : ''),
            kind,
          ),
        );
      }
    }
    if (id == null) pickPlace(widget.place ?? 'personal:');
    final prefill = widget.prefill;
    // A new password for this login (a sign-in Android's autofill offered to save).
    if (id != null && item != null && prefill != null) {
      password.text = (prefill['password'] ?? '') as String;
    }
    if (id == null && prefill != null) {
      name.text = (prefill['name'] ?? '') as String;
      username.text = (prefill['username'] ?? '') as String;
      password.text = (prefill['password'] ?? '') as String;
      final uri = (prefill['uri'] ?? '') as String;
      if (uri.isNotEmpty) uris.add(TextEditingController(text: uri));
    }
    if (uris.isEmpty && kind == 'login') uris.add(TextEditingController());
  }

  /// The Folder picker's value: `personal:` and the folder's id (none for
  /// no folder), or `org:`, the organization's id, `:`, and the shared
  /// folder's id.
  String get place => organizationId == null
      ? 'personal:${folderId ?? ''}'
      : 'org:$organizationId:${collectionId ?? ''}';

  /// Puts the item where the picker says.
  void pickPlace(String value) {
    final parts = value.split(':');
    if (parts.first == 'personal') {
      organizationId = null;
      collectionId = null;
      folderId = parts[1].isEmpty ? null : parts[1];
    } else if (parts.first == 'org' && parts.length == 3) {
      organizationId = parts[1];
      collectionId = parts[2];
      // An organization's item keeps the person's own folder as it was.
      if (!editing) folderId = null;
    }
  }

  /// The picker can't change anything: an organization's item the person
  /// may not move between its shared folders (share; the server refuses
  /// it too).
  bool get placeLocked =>
      editing && organizationId != null && permissions['share'] == false;

  /// The Folder picker's choices, grouped by owner: (value, label, glyph,
  /// depth), a heading's value starting with "heading:". An item stays with
  /// its owner (moving it between owners is the web vault's), and goes only
  /// into shared folders the person may add items to (or is in already).
  List<(String, String, String, int)> get placeOptions {
    final options = <(String, String, String, int)>[];
    if (!editing || organizationId == null) {
      options.add(('heading:personal', 'Personal', Glyphs.person, 0));
      options.add(('personal:', 'No folder', Glyphs.folder, 1));
      for (final f in widget.places.folders) {
        options.add((
          'personal:${f['id']}',
          f['label'] as String,
          Glyphs.folder,
          1 + (f['depth'] as int),
        ));
      }
    }
    for (final o in widget.places.organizations) {
      if (editing && o['id'] != organizationId) continue;
      final shared = (o['collections'] as List)
          .cast<Map<String, dynamic>>()
          .where((c) => c['canCreate'] == true || held.contains(c['id']))
          .toList();
      if (shared.isEmpty) continue;
      options.add((
        'heading:${o['id']}',
        o['name'] as String,
        Glyphs.building,
        0,
      ));
      for (final c in shared) {
        options.add((
          'org:${o['id']}:${c['id']}',
          c['label'] as String,
          Glyphs.sharedFolder,
          1 + (c['depth'] as int),
        ));
      }
    }
    // An item somewhere the person can't see listed still shows where it is.
    if (!options.any((o) => o.$1 == place)) {
      options.add((place, 'Where it is now', Glyphs.folder, 1));
    }
    return options;
  }

  /// The other shared folders an organization's item is in, which the
  /// picker leaves as they are.
  List<String> get alsoIn => [
    for (final c in held)
      if (c != collectionId)
        widget.places.collection(c)?['name'] as String? ?? 'a shared folder',
  ];

  /// The shared folders it's to be in: the one picked in place of the one
  /// it showed, and the rest as they are.
  List<String> get collectionIds {
    if (organizationId == null || collectionId == null) return const [];
    if (!editing) return [collectionId!];
    final shown =
        held.where((c) => widget.places.collection(c) != null).firstOrNull ??
        held.firstOrNull;
    return [
      collectionId!,
      for (final c in held)
        if (c != shown && c != collectionId) c,
    ];
  }

  Future<void> save() async {
    setState(() {
      working = true;
      error = null;
    });
    final draft = {
      'note': note,
      if (kind == 'card')
        'card': {
          for (final e in card.entries) e.key: e.value.text.trim(),
          'brand': brand,
          'expMonth': expMonth,
        },
      if (kind == 'identity')
        'identity': {
          for (final e in identity.entries) e.key: e.value.text.trim(),
        },
      'name': name.text.trim(),
      'username': username.text.trim(),
      'password': password.text,
      'totp': totp.text.trim(),
      'notes': notes.text,
      'uris': [for (final u in uris) u.text.trim()],
      'fields': [
        for (final f in fields)
          if (f.name.text.trim().isNotEmpty)
            {
              'name': f.name.text.trim(),
              'value': f.boolean ? (f.yes ? 'true' : 'false') : f.value.text,
              'type': f.kind,
            },
      ],
      'folderId': folderId,
      'organizationId': organizationId,
      'collectionIds': collectionIds,
      'favorite': favorite,
      'grant': ?grant,
      if (template != null)
        'template': {'id': template!['id'], 'name': template!['name']},
    };
    final answer = await whileBusy(() => Core.save(widget.id, draft));
    if (!mounted) return;
    if (answer.ok) {
      Navigator.of(context).pop(widget.id ?? '');
    } else {
      setState(() {
        working = false;
        error = answer.error;
      });
    }
  }

  /// A text field for a detail; secrets are hidden until shown.
  Widget _detail(
    TextEditingController controller,
    String label, {
    bool secret = false,
    TextInputType? keyboard,
  }) => TextField(
    controller: controller,
    obscureText: secret && hidden,
    keyboardType: keyboard,
    decoration: InputDecoration(
      labelText: label,
      suffixIcon: secret
          ? IconButton(
              tooltip: hidden ? 'Show' : 'Hide',
              icon: Icon(
                hidden
                    ? Icons.visibility_outlined
                    : Icons.visibility_off_outlined,
              ),
              onPressed: () => setState(() => hidden = !hidden),
            )
          : null,
    ),
  );

  List<Widget> _cardFields(Widget Function([double]) gap) => [
    gap(),
    _detail(card['cardholderName']!, 'Cardholder name'),
    gap(),
    DropdownButtonFormField<String?>(
      initialValue: brand,
      isExpanded: true,
      decoration: const InputDecoration(labelText: 'Brand'),
      items: [
        const DropdownMenuItem(value: null, child: Text('None')),
        for (final b in {..._brands, ?brand})
          DropdownMenuItem(value: b, child: Text(b)),
      ],
      onChanged: (v) => setState(() => brand = v),
    ),
    gap(),
    _detail(
      card['number']!,
      'Number',
      secret: true,
      keyboard: TextInputType.number,
    ),
    gap(),
    _sideBySide([
      DropdownButtonFormField<String?>(
        initialValue: expMonth,
        isExpanded: true,
        decoration: const InputDecoration(labelText: 'Expiration month'),
        items: [
          const DropdownMenuItem(value: null, child: Text('None')),
          for (var m = 1; m <= 12; m++)
            DropdownMenuItem(
              value: '$m',
              child: Text(m.toString().padLeft(2, '0')),
            ),
        ],
        onChanged: (v) => setState(() => expMonth = v),
      ),
      _detail(
        card['expYear']!,
        'Expiration year',
        keyboard: TextInputType.number,
      ),
    ]),
    gap(),
    _detail(
      card['code']!,
      'Security code',
      secret: true,
      keyboard: TextInputType.number,
    ),
  ];

  /// Two fields side by side where there's room for both labels, else one
  /// over the other.
  Widget _sideBySide(List<Widget> fields) => LayoutBuilder(
    builder: (context, constraints) {
      final roomy =
          constraints.maxWidth / MediaQuery.textScalerOf(context).scale(1) >=
          380;
      if (!roomy) {
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (var i = 0; i < fields.length; i++) ...[
              if (i > 0) const SizedBox(height: 12),
              fields[i],
            ],
          ],
        );
      }
      return Row(
        children: [
          for (var i = 0; i < fields.length; i++) ...[
            if (i > 0) const SizedBox(width: 12),
            Expanded(child: fields[i]),
          ],
        ],
      );
    },
  );

  List<Widget> _identityFields(Widget Function([double]) gap) => [
    for (final part in _identityParts.entries) ...[
      gap(),
      _detail(
        identity[part.key]!,
        part.value,
        secret: _identitySecrets.contains(part.key),
        keyboard: switch (part.key) {
          'email' => TextInputType.emailAddress,
          'phone' => TextInputType.phone,
          _ => null,
        },
      ),
    ],
  ];

  /// What the editor's title calls the item: "New Wi-Fi network", "Edit login".
  String get title {
    final what =
        (template?['name'] as String?) ??
        templateName ??
        (kind == 'note' ? 'Secure note' : ItemKind.of(kind).one);
    // "Wi-Fi network" and "API key" keep their capitals; "Login" doesn't.
    final first = what.split(' ').first;
    final keep =
        first.length > 1 &&
        first.substring(1).toLowerCase() != first.substring(1);
    final named = keep || what.isEmpty
        ? what
        : what[0].toLowerCase() + what.substring(1);
    return editing ? 'Edit $named' : 'New $named';
  }

  /// A custom field, as the web vault's form has it: its value under its
  /// name (a switch for yes or no), with its menu: rename it, change its
  /// kind, or remove it.
  Widget customField(int i) {
    final f = fields[i];
    final menu = PopupMenuButton<String>(
      tooltip: 'More for ${f.name.text}',
      icon: const Icon(Icons.more_vert_rounded),
      onSelected: (action) async {
        switch (action) {
          case 'rename':
            final named = await askFieldName(
              context,
              'Rename field',
              f.name.text,
            );
            if (named != null && mounted) setState(() => f.name.text = named);
          case 'remove':
            setState(() => fields.removeAt(i));
          default:
            final k = int.parse(action);
            setState(() {
              if (k == 2 && f.kind != 2) f.yes = f.value.text == 'true';
              if (k != 2 && f.kind == 2) {
                f.value.text = f.yes ? 'true' : 'false';
              }
              f.kind = k;
            });
        }
      },
      itemBuilder: (_) => [
        const PopupMenuItem(value: 'rename', child: Text('Rename')),
        for (final e in _fieldKinds.entries)
          CheckedPopupMenuItem(
            value: '${e.key}',
            checked: f.kind == e.key,
            child: Text(e.value),
          ),
        const PopupMenuItem(value: 'remove', child: Text('Remove')),
      ],
    );
    if (f.boolean) {
      return Row(
        children: [
          Expanded(
            child: SwitchListTile(
              value: f.yes,
              onChanged: (v) => setState(() => f.yes = v),
              title: Text(f.name.text),
              contentPadding: const EdgeInsets.only(left: 4),
            ),
          ),
          menu,
        ],
      );
    }
    return Row(
      children: [
        Expanded(
          child: TextField(
            controller: f.value,
            obscureText: f.hidden && hidden,
            decoration: InputDecoration(
              labelText: f.name.text,
              suffixIcon: f.hidden
                  ? IconButton(
                      tooltip: hidden ? 'Show' : 'Hide',
                      icon: Icon(
                        hidden
                            ? Icons.visibility_outlined
                            : Icons.visibility_off_outlined,
                      ),
                      onPressed: () => setState(() => hidden = !hidden),
                    )
                  : null,
            ),
          ),
        ),
        const SizedBox(width: 4),
        menu,
      ],
    );
  }

  /// Adds a field of a kind, once it has a name.
  Future<void> addField(int kind) async {
    final named = await askFieldName(context, 'Add a field');
    if (named == null || !mounted) return;
    setState(() => fields.add(_Field(named, kind == 2 ? 'false' : '', kind)));
  }

  /// One Folder picker, grouped by owner, as the web vault has it.
  Widget folderPicker() {
    final c = context.colors;
    final options = placeOptions;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        DropdownButtonFormField<String>(
          key: const ValueKey('folder-picker'),
          initialValue: place,
          isExpanded: true,
          decoration: const InputDecoration(labelText: 'Folder'),
          items: [
            for (final (value, label, glyph, depth) in options)
              DropdownMenuItem(
                value: value,
                enabled: !value.startsWith('heading:'),
                child: Padding(
                  padding: EdgeInsets.only(left: 18.0 * depth),
                  child: Row(
                    children: [
                      Glyph(
                        glyph,
                        size: 18,
                        color: value.startsWith('heading:')
                            ? Theme.of(context).colorScheme.onSurface
                            : c.muted,
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: Text(
                          label,
                          overflow: TextOverflow.ellipsis,
                          style: value.startsWith('heading:')
                              ? const TextStyle(fontWeight: FontWeight.w700)
                              : null,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
          ],
          onChanged: placeLocked
              ? null
              : (v) {
                  if (v == null || v.startsWith('heading:')) return;
                  setState(() => pickPlace(v));
                },
        ),
        if (alsoIn.isNotEmpty || placeLocked)
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 6, 4, 0),
            child: Text(
              [
                if (alsoIn.isNotEmpty) 'Also in ${alsoIn.join(', ')}.',
                if (placeLocked) 'Your organization doesn\'t let you move it to another shared folder.',
              ].join(' '),
              style: TextStyle(color: c.muted, fontSize: 13),
            ),
          ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget gap([double h = 12]) => SizedBox(height: h);
    Widget label(String text) => Text(
      text,
      style: TextStyle(
        color: c.muted,
        fontSize: 12,
        fontWeight: FontWeight.w600,
      ),
    );
    final primary = Theme.of(context).colorScheme.primary;
    return Scaffold(
      appBar: AppBar(
        title: Text(title),
        automaticallyImplyLeading: false,
        leading: IconButton(
          tooltip: 'Cancel',
          icon: const Icon(Icons.close_rounded),
          onPressed: () => Navigator.of(context).pop(),
        ),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: 12),
            child: WorkingButton(
              label: 'Save',
              working: working,
              onPressed: save,
            ),
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(20, 8, 20, 28),
        children: [
          if (error != null) ...[
            Text(
              error!,
              style: TextStyle(color: Theme.of(context).colorScheme.error),
            ),
            gap(),
          ],
          TextField(
            controller: name,
            autofocus: !editing,
            decoration: const InputDecoration(labelText: 'Name'),
          ),
          if (kind == 'card') ..._cardFields(gap),
          if (kind == 'identity') ..._identityFields(gap),
          if (kind == 'login') ...[
            gap(),
            TextField(
              controller: username,
              decoration: const InputDecoration(labelText: 'Username'),
            ),
            gap(),
            TextField(
              controller: password,
              obscureText: hidden,
              decoration: InputDecoration(
                labelText: 'Password',
                suffixIcon: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton(
                      tooltip: hidden ? 'Show' : 'Hide',
                      icon: Icon(
                        hidden
                            ? Icons.visibility_outlined
                            : Icons.visibility_off_outlined,
                      ),
                      onPressed: () => setState(() => hidden = !hidden),
                    ),
                    IconButton(
                      tooltip: 'Generate a password',
                      icon: const Icon(Icons.casino_outlined),
                      onPressed: () async {
                        final made = await openGenerator(context, widget.core);
                        if (made == null || !mounted) return;
                        setState(() {
                          password.text = made;
                          hidden = false;
                        });
                      },
                    ),
                  ],
                ),
              ),
            ),
            gap(),
            TextField(
              controller: totp,
              decoration: const InputDecoration(
                labelText: 'One-time code secret (optional)',
                hintText: 'The key or otpauth:// address from the site',
              ),
            ),
            gap(16),
            label('Websites'),
            for (var i = 0; i < uris.length; i++) ...[
              gap(6),
              TextField(
                controller: uris[i],
                keyboardType: TextInputType.url,
                decoration: InputDecoration(
                  hintText: 'https://example.com',
                  suffixIcon: IconButton(
                    tooltip: 'Remove',
                    icon: const Icon(Icons.remove_circle_outline),
                    onPressed: () => setState(() => uris.removeAt(i)),
                  ),
                ),
              ),
            ],
            Align(
              alignment: Alignment.centerLeft,
              child: TextButton.icon(
                onPressed: () =>
                    setState(() => uris.add(TextEditingController())),
                icon: const Icon(Icons.add_rounded),
                label: const Text('Add a website'),
              ),
            ),
          ],
          gap(16),
          // A template's fields, under its name, as in the web vault.
          label(
            template != null || templateName != null
                ? '${template?['name'] ?? templateName} details'
                : 'Custom fields',
          ),
          for (var i = 0; i < fields.length; i++) ...[gap(6), customField(i)],
          Align(
            alignment: Alignment.centerLeft,
            child: PopupMenuButton<int>(
              tooltip: 'Add a field',
              onSelected: addField,
              itemBuilder: (_) => [
                for (final e in _fieldKinds.entries)
                  PopupMenuItem(value: e.key, child: Text(e.value)),
              ],
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 10),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(Icons.add_rounded, size: 18, color: primary),
                    const SizedBox(width: 8),
                    Text(
                      'Add a field',
                      style: TextStyle(
                        color: primary,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
          gap(),
          TextField(
            controller: notes,
            minLines: 3,
            maxLines: 8,
            decoration: const InputDecoration(labelText: 'Notes'),
          ),
          gap(16),
          folderPicker(),
          gap(),
          SwitchListTile(
            value: favorite,
            onChanged: (v) => setState(() => favorite = v),
            title: const Text('Favorite'),
            contentPadding: EdgeInsets.zero,
          ),
        ],
      ),
    );
  }
}

/// The kinds of custom field, as the editor offers them.
const _fieldKinds = {0: 'Text', 1: 'Hidden', 2: 'Yes or no'};

/// A field's name, asked for: null when cancelled.
Future<String?> askFieldName(
  BuildContext context,
  String title, [
  String name = '',
]) => showDialog<String>(
  context: context,
  builder: (context) => _FieldNameDialog(title: title, name: name),
);

class _FieldNameDialog extends StatefulWidget {
  const _FieldNameDialog({required this.title, required this.name});

  final String title;
  final String name;

  @override
  State<_FieldNameDialog> createState() => _FieldNameDialogState();
}

class _FieldNameDialogState extends State<_FieldNameDialog> {
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

/// Card brands, as upstream's apps name them.
const _brands = [
  'Visa',
  'Mastercard',
  'Amex',
  'Discover',
  'Diners Club',
  'JCB',
  'Maestro',
  'UnionPay',
  'RuPay',
  'Other',
];

/// An identity's details, by upstream's names, with what the editor calls them.
const _identityParts = {
  'title': 'Title',
  'firstName': 'First name',
  'middleName': 'Middle name',
  'lastName': 'Last name',
  'username': 'Username',
  'company': 'Company',
  'email': 'Email',
  'phone': 'Phone',
  'address1': 'Address',
  'address2': 'Address line 2',
  'address3': 'Address line 3',
  'city': 'City or town',
  'state': 'State or province',
  'postalCode': 'Postal code',
  'country': 'Country',
  'ssn': 'Social Security number',
  'passportNumber': 'Passport number',
  'licenseNumber': 'License number',
};

/// The identity details that are secrets, revealed into the editor.
const _identitySecrets = {'ssn', 'passportNumber', 'licenseNumber'};
