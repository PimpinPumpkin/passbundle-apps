// Adding and editing an item. Editing shows its secrets in the fields, so
// they pass through Dart while the editor is open, as typing them does; the
// core encrypts what's saved (docs/core.md, "Memory"). A new item comes from
// the New item picker (new_item.dart): a kind, or a template with its
// fields. Where it goes is one Folder picker, grouped by owner, as the web
// vault has it: Personal and its folders, then each organization's shared
// folders the person may add items to. Where an organization keeps items
// out of Personal, a new item is offered only those shared folders and
// starts where the core starts one (places.dart, `newItemStart`); with none,
// the vault opens no editor (vault.dart). Closing it with anything changed
// (Cancel, Escape, or Android's Back) asks first, as the web vault does; a
// click outside it never closes it. A template's fields have kinds beyond
// upstream's (a date, a website, an email address, a phone number, a
// number, a one-time code), which the editor uses and keeps in the
// template's definition or an item's marker (core/src/templates.rs; the meta
// repo's DESIGN.md, "Item templates"); a date can be the item's expiration
// date, which keeps "Expires" the same as it.
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'generator.dart';
import 'glyphs.dart';
import 'new_item.dart';
import 'places.dart';
import 'theme.dart';
import 'widgets.dart';

/// What the editor saved: the item's id ('' for a new one), and whether the
/// sync after saving came through. When it didn't, the item is saved all the
/// same and shows on this device (core/src/app.rs, `save`): the editor
/// closes, so it's never saved twice.
class EditorSaved {
  const EditorSaved(this.id, {this.synced = true});

  final String id;
  final bool synced;
}

/// Opens the editor; resolves to what was saved, or null if canceled. A
/// new item is the kind or template `choice` picked, going to `place` (as
/// [_ItemEditorState.place] says it) unless the person picks another.
Future<EditorSaved?> openEditor(
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
    return showDialog<EditorSaved>(
      context: context,
      // Only Cancel, Escape, and Save close it: a click beside it could
      // throw away what was typed.
      barrierDismissible: false,
      builder: (context) => Dialog(
        clipBehavior: Clip.antiAlias,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 560, maxHeight: 760),
          // Notices of its own: the vault's (a sync's, a copy's) stay with
          // the vault under the dialog, instead of floating over the
          // editor's last rows, where they'd take the taps meant for them.
          child: ScaffoldMessenger(child: editor),
        ),
      ),
    );
  }
  return Navigator.of(context).push<EditorSaved>(
    MaterialPageRoute(fullscreenDialog: true, builder: (_) => editor),
  );
}

/// A custom field as the editor has it: upstream's type (0 text, 1 hidden,
/// 2 yes or no), its name, and its value ("true" or "false" for yes or no);
/// and the kind its template gives it, if any (a date, a website, and so
/// on), and whether it's the expiration date.
class _Field {
  _Field(
    String name,
    String value,
    this.kind, {
    this.typed,
    this.expires = false,
  }) : name = TextEditingController(text: name),
      value = TextEditingController(text: value),
      yes = value == 'true';

  /// A field of a template's kind: its type kept as that kind is.
  _Field.of(String name, String value, String kind, {bool expires = false})
    : this(
        name,
        value,
        storedType(kind),
        typed: typedKinds.contains(kind) ? kind : null,
        expires: kind == 'date' && expires,
      );

  final TextEditingController name;
  final TextEditingController value;
  int kind;
  bool yes;

  /// date, url, email, phone, number, or totp: a template's kind for it.
  String? typed;

  /// A date that's the item's expiration date (a template's: new items').
  bool expires;

  bool get hidden => kind == 1;
  bool get boolean => kind == 2;

  /// Its kind, as templates name them.
  String get kindName =>
      typed ??
      switch (kind) {
        1 => 'hidden',
        2 => 'boolean',
        _ => 'text',
      };
}

/// The kinds a template gives a field beyond upstream's text, hidden, and
/// yes or no (core/src/templates.rs, `FieldKind`).
const typedKinds = {'date', 'url', 'email', 'phone', 'number', 'totp'};

/// Upstream's field type a kind is kept as: a one-time code's key is hidden.
int storedType(String kind) => switch (kind) {
  'hidden' || 'totp' => 1,
  'boolean' => 2,
  _ => 0,
};

/// "YYYY-MM-DD", a real day, as the core reads a date.
bool isIsoDate(String text) {
  final match = RegExp(r'^(\d{4})-(\d{2})-(\d{2})$').firstMatch(text);
  if (match == null) return false;
  final year = int.parse(match[1]!);
  final month = int.parse(match[2]!);
  final day = int.parse(match[3]!);
  final date = DateTime(year, month, day);
  return date.year == year && date.month == month && date.day == day;
}

/// What's wrong with a typed field's value, said plainly, or null: the
/// editor shows it under the field, and it never stops the item saving. An
/// empty field is fine. The web vault's words (templates.ts,
/// `typedFieldProblem`).
String? typedFieldProblem(String? kind, String value) {
  final v = value.trim();
  if (v.isEmpty) return null;
  switch (kind) {
    case 'date':
      return isIsoDate(v)
          ? null
          : 'Write the date as year, month, and day, such as 2027-03-31.';
    case 'url':
      final uri = Uri.tryParse(v);
      final web =
          !v.contains(RegExp(r'\s')) &&
          ((uri != null &&
                  (uri.scheme == 'https' || uri.scheme == 'http') &&
                  uri.host.contains('.')) ||
              RegExp(
                r'^[^/:@]+\.[a-z]{2,}(:\d+)?(/.*)?$',
                caseSensitive: false,
              ).hasMatch(v));
      return web ? null : "This doesn't look like a web address.";
    case 'email':
      return RegExp(r'^[^\s@]+@[^\s@]+\.[^\s@]+$').hasMatch(v)
          ? null
          : "This doesn't look like an email address.";
    case 'phone':
      return RegExp(r'^\+?[\d\s().\-/]+$').hasMatch(v) &&
              RegExp(r'\d').allMatches(v).length >= 3
          ? null
          : "This doesn't look like a phone number.";
    case 'number':
      return RegExp(r'^[+-]?(\d+|\d{1,3}(,\d{3})+)(\.\d+)?$').hasMatch(v)
          ? null
          : "This isn't a number.";
    default:
      return null;
  }
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
  /// its new password. Or a new card's or identity's name and parts (`kind`
  /// "card" or "identity"), from a payment or address form Android's
  /// autofill offered to save.
  final Map<String, dynamic>? prefill;

  /// Tests only: what Save sends the core, as it sends it (the tests' vault
  /// has no server to save to).
  @visibleForTesting
  static void Function(Map<String, dynamic> draft)? debugSaving;

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

  /// When it expires ("YYYY-MM-DD", empty for never), and when it reminds:
  /// as `keykeeper.remind` holds it, empty for the default
  /// (core/src/expiration.rs), with the item's own days in words.
  final expires = TextEditingController();
  String remind = '';
  String? remindWords;

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

  /// The item asks for the master password before its secrets are shown,
  /// copied, or filled (upstream's `reprompt`), as the web vault's editor
  /// sets it.
  bool reprompt = false;

  /// Whether the person may change the item's websites: always for a new
  /// item, and for an existing one only when they can see its passwords.
  bool get websitesEditable => !editing || permissions['seePasswords'] == true;
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

  /// A custom template being made ("login" or "note", what its new items
  /// will be), or being edited (its definition's base).
  String? templateBase;

  /// It's a custom template: its fields' kinds go in its definition, and a
  /// date in it is a default for new items, never its own expiration.
  bool get isTemplate => templateBase != null;

  /// It keeps its fields' kinds: a template, or an item from one (in its
  /// marker). An item from no template has nowhere to keep them.
  bool get keepsKinds =>
      isTemplate || template != null || templateName != null;

  /// The field that's the item's expiration date, as its template marks it:
  /// "Expires" and it stay the same. A template's is a default instead.
  _Field? get expirationField => isTemplate
      ? null
      : fields
            .where((f) => f.expires && f.typed == 'date' && f.kind == 0)
            .firstOrNull;

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
      templateBase = (item['definition'] as Map?)?['base'] as String?;
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
        // Its kind, from the item's template (core/src/templates.rs).
        final typed = f['kind'] as String?;
        fields.add(
          _Field(
            f['name'] as String,
            (hidden ? reveal(id, 'field:$i') : f['value'] as String?) ?? '',
            (f['type'] as int?) ?? (hidden ? 1 : 0),
            typed: typedKinds.contains(typed) ? typed : null,
            expires: f['expires'] == true,
          ),
        );
      }
      favorite = item['favorite'] == true;
      reprompt = item['reprompt'] == true;
      expires.text = (item['expires'] ?? '') as String;
      // The template's expiration date wins while it holds a date, as the
      // core reads it (core/src/expiration.rs, `stored`).
      final linked = expirationField?.value.text.trim() ?? '';
      if (isIsoDate(linked)) expires.text = linked;
      final itemRemind = item['remind'] as Map?;
      remind = (itemRemind?['value'] as String?) ?? '';
      remindWords = itemRemind?['words'] as String?;
      folderId = item['folderId'] as String?;
      organizationId = item['organizationId'] as String?;
      held = (item['collectionIds'] as List?)?.cast<String>() ?? const [];
      // The first of its shared folders the person has.
      collectionId = shown;
    }
    // A new item: the kind or template picked, where it's going.
    final choice = widget.choice;
    if (id == null && choice != null) {
      kind = choice.kind;
      template = choice.template;
      templateBase = choice.newTemplate;
      if (choice.name != null) name.text = choice.name!;
      // A template's fields with their kinds and defaults; a new template's
      // ("Save as template") with their kinds only.
      for (final f in [
        ...((template?['fields'] as List?) ?? const [])
            .cast<Map<String, dynamic>>(),
        ...choice.fields,
      ]) {
        final kind = (f['kind'] as String?) ?? 'text';
        fields.add(
          _Field.of(
            f['label'] as String,
            (f['value'] as String?) ?? (kind == 'boolean' ? 'false' : ''),
            kind,
            expires: f['expires'] == true,
          ),
        );
      }
      // Its template's expiration date, when the template gives one.
      final linked = expirationField?.value.text.trim() ?? '';
      if (isIsoDate(linked)) expires.text = linked;
    }
    if (id == null) pickPlace(startPlace);
    final prefill = widget.prefill;
    // A new password for this login (a sign-in Android's autofill offered to save).
    if (id != null && item != null && prefill != null) {
      password.text = (prefill['password'] ?? '') as String;
    }
    if (id == null && prefill != null) {
      name.text = (prefill['name'] ?? '') as String;
      String part(String key) => (prefill[key] ?? '') as String;
      switch (prefill['kind']) {
        case 'card':
          kind = 'card';
          for (final key in card.keys) {
            card[key]!.text = part(key);
          }
          brand = prefill['brand'] as String?;
          expMonth = prefill['expMonth'] as String?;
        case 'identity':
          kind = 'identity';
          for (final key in identity.keys) {
            identity[key]!.text = part(key);
          }
        default:
          username.text = part('username');
          password.text = part('password');
          final uri = part('uri');
          if (uri.isNotEmpty) uris.add(TextEditingController(text: uri));
      }
    }
    if (uris.isEmpty && kind == 'login') uris.add(TextEditingController());
    _opened = _fingerprint();
  }

  /// The Folder picker's value: `personal:` and the folder's id (none for
  /// no folder), or `org:`, the organization's id, `:`, and the shared
  /// folder's id.
  String get place => organizationId == null
      ? 'personal:${folderId ?? ''}'
      : 'org:$organizationId:${collectionId ?? ''}';

  /// Where a new item starts: the place it was opened for, where a new item
  /// may go there, else where the core starts one: Personal, or a shared
  /// folder where an organization keeps items out of Personal.
  String get startPlace {
    final places = widget.places;
    final asked = widget.place;
    if (asked != null &&
        (asked.startsWith('personal:')
            ? places.newInPersonal
            : places.collection(asked.split(':').last)?['canCreate'] == true)) {
      return asked;
    }
    final start = places.newItemStart;
    return start == null ? 'personal:' : 'org:${start.$1}:${start.$2}';
  }

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

  /// The shared folder the picker shows an organization's item in: the
  /// first of its folders the person has.
  String? get shown =>
      held.where((c) => widget.places.collection(c) != null).firstOrNull ??
      held.firstOrNull;

  /// The picker can't change anything: an organization's item the person
  /// may not move between its shared folders, as the server has it (edit
  /// and share on the item, and share in the folder it leaves; the server
  /// refuses it too).
  bool get placeLocked {
    if (!editing || organizationId == null) return false;
    if (permissions['edit'] != true || permissions['share'] != true) {
      return true;
    }
    final from = shown == null ? null : widget.places.collection(shown!);
    return (from?['grid'] as Map?)?['share'] != true;
  }

  /// The Folder picker's choices, grouped by owner: (value, label, glyph,
  /// depth), a heading's value starting with "heading:". An item stays with
  /// its owner (moving it between owners is the web vault's), and goes only
  /// into shared folders the person may add items to (or is in already). A
  /// new item goes in Personal only where the core says it may.
  List<(String, String, String, int)> get placeOptions {
    final options = <(String, String, String, int)>[];
    if (editing ? organizationId == null : widget.places.newInPersonal) {
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
    return [
      collectionId!,
      for (final c in held)
        if (c != shown && c != collectionId) c,
    ];
  }

  /// What saving sends: the item as the fields have it now.
  Map<String, dynamic> get draft => {
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
        // Names as they are: one with spaces around it is the web
        // vault's too, and stays so.
        if (f.name.text.trim().isNotEmpty)
          {
            'name': f.name.text,
            'value': f.boolean ? (f.yes ? 'true' : 'false') : f.value.text,
            'type': f.kind,
          },
    ],
    'folderId': folderId,
    'organizationId': organizationId,
    'collectionIds': collectionIds,
    'favorite': favorite,
    'reprompt': reprompt,
    // An empty date clears it and its reminder days (core/src/ffi.rs,
    // pb_save). A card's date is its expiration month and year: a draft
    // without these keeps any other the card has as it is.
    if (kind != 'card') ...{'expires': expires.text.trim(), 'remind': remind},
    'grant': ?grant,
    if (template != null)
      'template': {'id': template!['id'], 'name': template!['name']},
    // The fields' kinds, for the template's definition or the item's
    // marker (core/src/templates.rs, `Typing`).
    if (keepsKinds) 'typing': typing,
    // "Create template" and "Save as template": a new custom template.
    if (!editing && isTemplate) 'newTemplate': templateBase,
  };

  /// The fields' kinds beyond upstream's, by name, and which date is the
  /// expiration date: `{types, expires}`.
  Map<String, dynamic> get typing {
    final named = [
      for (final f in fields)
        if (f.typed != null && f.name.text.trim().isNotEmpty) f,
    ];
    final expiring = named
        .where((f) => f.typed == 'date' && f.expires)
        .firstOrNull;
    return {
      'types': {for (final f in named) f.name.text: f.typed},
      'expires': ?expiring?.name.text,
    };
  }

  /// The draft as the editor opened, as a fingerprint rather than a copy,
  /// so its secrets aren't kept twice: whether anything changed since.
  late final int _opened;
  int _fingerprint() => jsonEncode(draft).hashCode;
  bool get changed => _fingerprint() != _opened;

  /// Cancel, Escape, or Android's Back: closes it as it is when nothing
  /// changed, else once the person says to discard the changes.
  Future<void> close() async {
    if (changed && !await askToDiscard(context)) return;
    if (mounted) Navigator.of(context).pop();
  }

  Future<void> save() async {
    setState(() {
      working = true;
      error = null;
    });
    final draft = this.draft;
    ItemEditor.debugSaving?.call(draft);
    final answer = await whileBusy(() => Core.save(widget.id, draft));
    if (!mounted) return;
    if (answer.ok) {
      Navigator.of(context).pop(
        EditorSaved(widget.id ?? '', synced: answer.body['synced'] != false),
      );
    } else {
      setState(() {
        working = false;
        // Offline, nothing was saved: what's typed stays, for another try.
        error = answer.offline
            ? 'Not saved. ${answer.error} What you typed is still here.'
            : answer.error;
      });
    }
  }

  /// "Remind me"'s choices: (what the field holds, what it says). The
  /// item's own days, when they aren't one of these, stay a choice.
  List<(String, String)> get remindOptions => [
    ..._remindOptions,
    if (!_remindOptions.any((o) => o.$1 == remind))
      (remind, remindWords ?? remind),
  ];

  /// The calendar, for the expiration date.
  Future<void> pickExpiry() async {
    final picked = await pickDate(expires.text);
    if (picked == null || !mounted) return;
    setState(() {
      expires.text = picked;
      expiresChanged();
    });
  }

  /// The calendar, from the date [shown] if it's one: the day picked, as
  /// "YYYY-MM-DD", or null.
  Future<String?> pickDate(String shown) async {
    final now = DateTime.now();
    final first = DateTime(now.year - 20);
    final last = DateTime(now.year + 50, 12, 31);
    final from = isIsoDate(shown.trim()) ? DateTime.parse(shown.trim()) : null;
    final picked = await showDatePicker(
      context: context,
      initialDate: from != null && !from.isBefore(first) && !from.isAfter(last)
          ? from
          : now,
      firstDate: first,
      lastDate: last,
    );
    if (picked == null) return null;
    String two(int n) => n.toString().padLeft(2, '0');
    return '${picked.year}-${two(picked.month)}-${two(picked.day)}';
  }

  /// "Expires" changed: the template's expiration date field follows.
  void expiresChanged() {
    final field = expirationField;
    final date = expires.text.trim();
    if (field != null && field.value.text != date) field.value.text = date;
  }

  /// The template's expiration date field changed: "Expires" follows, with
  /// a date or none (not with anything else, which the field's warning is
  /// about).
  void expirationFieldChanged(_Field f) {
    if (f != expirationField) return;
    final date = f.value.text.trim();
    if ((date.isEmpty || isIsoDate(date)) && expires.text.trim() != date) {
      expires.text = date;
    }
  }

  /// What a field or a picker holds, in the same type either way (a picker's
  /// own is heavier).
  TextStyle? get _valueStyle => Theme.of(context).textTheme.bodyLarge;

  /// The buttons at a field's end: the last one's glyph as far from the
  /// box's edge as the text is from its start.
  Widget _trailing(List<Widget> buttons) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [...buttons, const SizedBox(width: 2)],
  );

  /// Shows or hides the secrets, as an item's page does.
  Widget _showOrHide() => IconButton(
    tooltip: hidden ? 'Show' : 'Hide',
    icon: Glyph(
      hidden ? Glyphs.eye : Glyphs.eyeOff,
      size: 20,
      color: context.colors.muted,
    ),
    onPressed: () => setState(() => hidden = !hidden),
  );

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
    // A secret isn't learned or suggested by the keyboard, as the master
    // password's field isn't.
    enableSuggestions: !secret,
    autocorrect: !secret,
    enableIMEPersonalizedLearning: !secret,
    decoration: labeledField(
      context,
      label,
      suffix: secret ? _trailing([_showOrHide()]) : null,
    ),
  );

  List<Widget> _cardFields(Widget Function([double]) gap) => [
    gap(),
    _detail(card['cardholderName']!, 'Cardholder name'),
    gap(),
    DropdownButtonFormField<String?>(
      initialValue: brand,
      isExpanded: true,
      style: _valueStyle,
      decoration: labeledField(context, 'Brand'),
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
        style: _valueStyle,
        decoration: labeledField(context, 'Expiration month'),
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
              if (i > 0) const SizedBox(height: _between),
              fields[i],
            ],
          ],
        );
      }
      return Row(
        children: [
          for (var i = 0; i < fields.length; i++) ...[
            if (i > 0) const SizedBox(width: _between),
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
    if (isTemplate) return editing ? 'Edit template' : 'New template';
    final what =
        (template?['name'] as String?) ??
        templateName ??
        ItemKind.of(kind).one;
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
    // A template's fields take any kind; an item's, upstream's three (a
    // kind its template gave it stays while its type does).
    final kinds = isTemplate
        ? _templateKinds
        : {for (final e in _fieldKinds.entries) _kindNames[e.key]!: e.value};
    final menu = PopupMenuButton<String>(
      tooltip: 'More for ${f.name.text}',
      icon: Icon(
        Icons.more_vert_rounded,
        size: 20,
        color: context.colors.muted,
      ),
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
          case 'expires':
            setState(() {
              final on = !f.expires;
              for (final other in fields) {
                other.expires = false;
              }
              f.expires = on;
            });
          default:
            final kind = action.substring('kind:'.length);
            final k = storedType(kind);
            setState(() {
              if (k == 2 && f.kind != 2) f.yes = f.value.text == 'true';
              if (k != 2 && f.kind == 2) {
                f.value.text = f.yes ? 'true' : 'false';
              }
              // An item's field keeps its template's kind while its type
              // stays the same.
              if (isTemplate || k != f.kind) {
                f.typed = typedKinds.contains(kind) ? kind : null;
              }
              if (f.typed != 'date') f.expires = false;
              f.kind = k;
            });
        }
      },
      itemBuilder: (_) => [
        const PopupMenuItem(value: 'rename', child: Text('Rename')),
        for (final e in kinds.entries)
          CheckedPopupMenuItem(
            value: 'kind:${e.key}',
            checked: isTemplate
                ? f.kindName == e.key
                : storedType(f.kindName) == storedType(e.key),
            child: Text(e.value),
          ),
        if (isTemplate && f.typed == 'date')
          CheckedPopupMenuItem(
            value: 'expires',
            checked: f.expires,
            child: const Text('New items expire on this date'),
          ),
        const PopupMenuItem(value: 'remove', child: Text('Remove')),
      ],
    );
    // In the field's box, at its end, where the other fields' buttons are:
    // every box is as wide as the next.
    if (f.boolean) {
      return FieldBox(
        child: Row(
          children: [
            Expanded(
              child: SwitchListTile(
                value: f.yes,
                onChanged: (v) => setState(() => f.yes = v),
                title: Text(f.name.text),
                contentPadding: const EdgeInsets.only(left: 16),
              ),
            ),
            menu,
            const SizedBox(width: 2),
          ],
        ),
      );
    }
    // A typed field's keyboard, its date picker, and under it a warning
    // that never stops a save, or what it is.
    final problem = typedFieldProblem(f.typed, f.value.text);
    final expiring = f.expires && f.typed == 'date';
    final String? note;
    if (isTemplate && f.typed != null) {
      final kindName = _templateKinds[f.typed];
      note = expiring
          ? '$kindName. New items expire on this date.'
          : '$kindName.';
    } else if (expiring) {
      note = 'Also when this item expires.';
    } else if (f.typed == 'totp') {
      note = 'A one-time code\'s key. The item shows its current code.';
    } else {
      note = null;
    }
    return TextField(
      controller: f.value,
      obscureText: f.hidden && hidden,
      enableSuggestions: !f.hidden && f.typed == null,
      autocorrect: !f.hidden && f.typed == null,
      enableIMEPersonalizedLearning: !f.hidden,
      keyboardType: switch (f.typed) {
        'date' => TextInputType.datetime,
        'url' => TextInputType.url,
        'email' => TextInputType.emailAddress,
        'phone' => TextInputType.phone,
        'number' => const TextInputType.numberWithOptions(
          signed: true,
          decimal: true,
        ),
        _ => null,
      },
      onChanged: (_) => setState(() => expirationFieldChanged(f)),
      decoration: labeledField(
        context,
        f.name.text,
        hint: f.typed == 'date' ? 'Such as 2027-03-31' : null,
        suffix: _trailing([
          if (f.hidden) _showOrHide(),
          if (f.typed == 'date')
            IconButton(
              tooltip: 'Pick a date for ${f.name.text}',
              icon: Icon(
                Icons.event_outlined,
                size: 20,
                color: context.colors.muted,
              ),
              onPressed: () async {
                final picked = await pickDate(f.value.text);
                if (picked == null || !mounted) return;
                setState(() {
                  f.value.text = picked;
                  expirationFieldChanged(f);
                });
              },
            ),
          menu,
        ]),
      ).copyWith(helperText: problem ?? note, helperMaxLines: 3),
    );
  }

  /// Adds a field of a kind (as templates name kinds), once it has a name.
  Future<void> addField(String kind) async {
    final named = await askFieldName(context, 'Add a field');
    if (named == null || !mounted) return;
    setState(
      () => fields.add(
        _Field.of(named, kind == 'boolean' ? 'false' : '', kind),
      ),
    );
  }

  /// A new item that can't go in Personal, as an organization keeps items
  /// out of it: the picker says so, as the import does.
  bool get keptOut => !editing && !widget.places.newInPersonal;

  /// One Folder picker, grouped by owner, as the web vault has it.
  Widget folderPicker() {
    final c = context.colors;
    final options = placeOptions;
    Widget option(String value, String label, String glyph, int depth) {
      final heading = value.startsWith('heading:');
      return Padding(
        padding: EdgeInsets.only(left: 18.0 * depth),
        child: Row(
          children: [
            Glyph(
              glyph,
              size: 18,
              color: heading
                  ? Theme.of(context).colorScheme.onSurface
                  : c.muted,
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                label,
                overflow: TextOverflow.ellipsis,
                style: heading
                    ? const TextStyle(fontWeight: FontWeight.w700)
                    : null,
              ),
            ),
          ],
        ),
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        DropdownButtonFormField<String>(
          key: const ValueKey('folder-picker'),
          initialValue: place,
          isExpanded: true,
          style: _valueStyle,
          decoration: labeledField(context, 'Folder'),
          // The one picked starts where the other fields' values do; in the
          // list, each is set in under its owner.
          selectedItemBuilder: (_) => [
            for (final (value, label, glyph, _) in options)
              option(value, label, glyph, 0),
          ],
          items: [
            for (final (value, label, glyph, depth) in options)
              DropdownMenuItem(
                value: value,
                enabled: !value.startsWith('heading:'),
                child: option(value, label, glyph, depth),
              ),
          ],
          onChanged: placeLocked
              ? null
              : (v) {
                  if (v == null || v.startsWith('heading:')) return;
                  setState(() => pickPlace(v));
                },
        ),
        if (alsoIn.isNotEmpty || placeLocked || keptOut)
          _Footnote(
            [
              if (alsoIn.isNotEmpty) 'Also in ${alsoIn.join(', ')}.',
              if (placeLocked) 'Your organization doesn\'t let you move it to another shared folder.',
              if (keptOut)
                '${widget.places.keepsItemsOut ?? 'Your organization'} keeps items out of Personal.',
            ].join(' '),
          ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final scheme = Theme.of(context).colorScheme;
    Widget gap([double h = _between]) => SizedBox(height: h);
    final editor = Scaffold(
      appBar: AppBar(
        title: Text(title),
        automaticallyImplyLeading: false,
        leading: IconButton(
          tooltip: 'Cancel',
          icon: const Icon(Icons.close_rounded),
          onPressed: () => Navigator.of(context).maybePop(),
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
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // Why it wasn't saved, over the fields rather than among them: it
          // shows wherever the form is scrolled to.
          if (error != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(
                _gutter + 4,
                0,
                _gutter + 4,
                8,
              ),
              child: Semantics(
                liveRegion: true,
                child: Text(
                  error!,
                  style: TextStyle(color: scheme.error, height: 1.35),
                ),
              ),
            ),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.fromLTRB(_gutter, 4, _gutter, 28),
              children: [
                // A custom template says so, as the web vault's editor does.
                if (isTemplate)
                  Padding(
                    padding: const EdgeInsets.fromLTRB(4, 4, 4, 14),
                    child: Text(
                      'This is a template for new ${templateBase == 'login' ? 'logins' : 'notes'}. '
                      'Its name is the template\'s name, and new items start with its fields. '
                      'Values you enter here become defaults.',
                      key: const ValueKey('template-hint'),
                      style: TextStyle(color: c.muted, height: 1.35),
                    ),
                  ),
                TextField(
                  controller: name,
                  autofocus: !editing,
                  decoration: labeledField(context, 'Name'),
                ),
                if (kind == 'card') ..._cardFields(gap),
                if (kind == 'identity') ..._identityFields(gap),
                if (kind == 'login') ...[
                  gap(),
                  TextField(
                    controller: username,
                    decoration: labeledField(context, 'Username'),
                  ),
                  gap(),
                  TextField(
                    controller: password,
                    obscureText: hidden,
                    enableSuggestions: false,
                    autocorrect: false,
                    enableIMEPersonalizedLearning: false,
                    decoration: labeledField(
                      context,
                      'Password',
                      suffix: _trailing([
                        _showOrHide(),
                        IconButton(
                          tooltip: 'Generate a password',
                          icon: Glyph(
                            Glyphs.generator,
                            size: 20,
                            color: c.muted,
                          ),
                          onPressed: () async {
                            final made = await openGenerator(
                              context,
                              widget.core,
                            );
                            if (made == null || !mounted) return;
                            setState(() {
                              password.text = made;
                              hidden = false;
                            });
                          },
                        ),
                      ]),
                    ),
                  ),
                  gap(),
                  TextField(
                    controller: totp,
                    obscureText: hidden,
                    enableSuggestions: false,
                    autocorrect: false,
                    enableIMEPersonalizedLearning: false,
                    decoration: labeledField(
                      context,
                      'One-time code secret (optional)',
                      hint: 'Setup key or otpauth:// link',
                      suffix: _trailing([_showOrHide()]),
                    ),
                  ),
                  const SectionHeading('Websites', padding: _heading),
                  for (var i = 0; i < uris.length; i++) ...[
                    if (i > 0) gap(),
                    TextField(
                      controller: uris[i],
                      // Someone who can't see the item's passwords can't change
                      // where autofill sends them (core: vault.rs, encrypt_item_from).
                      readOnly: !websitesEditable,
                      keyboardType: TextInputType.url,
                      decoration: labeledField(
                        context,
                        'Website',
                        hint: 'https://example.com',
                        suffix: websitesEditable
                            ? _trailing([
                                IconButton(
                                  tooltip: 'Remove',
                                  icon: Icon(
                                    Icons.remove_circle_outline,
                                    size: 20,
                                    color: c.muted,
                                  ),
                                  onPressed: () =>
                                      setState(() => uris.removeAt(i)),
                                ),
                              ])
                            : null,
                      ),
                    ),
                  ],
                  if (websitesEditable)
                    Align(
                      alignment: Alignment.centerLeft,
                      child: InkWell(
                        borderRadius: BorderRadius.circular(10),
                        onTap: () =>
                            setState(() => uris.add(TextEditingController())),
                        child: const _AddRow('Add a website'),
                      ),
                    )
                  else
                    const _Footnote(
                      "Someone who can see this item's passwords can change its websites.",
                    ),
                ],
                // A template's fields, under its name, as in the web vault.
                SectionHeading(
                  isTemplate
                      ? 'Fields for new items'
                      : template != null || templateName != null
                      ? '${template?['name'] ?? templateName} details'
                      : 'Custom fields',
                  padding: _heading,
                ),
                for (var i = 0; i < fields.length; i++) ...[
                  if (i > 0) gap(),
                  customField(i),
                ],
                Align(
                  alignment: Alignment.centerLeft,
                  child: PopupMenuButton<String>(
                    tooltip: 'Add a field',
                    onSelected: addField,
                    itemBuilder: (_) => [
                      // A template's fields take any kind.
                      for (final e
                          in (isTemplate
                                  ? _templateKinds
                                  : {
                                      for (final e in _fieldKinds.entries)
                                        _kindNames[e.key]!: e.value,
                                    })
                              .entries)
                        PopupMenuItem(value: e.key, child: Text(e.value)),
                    ],
                    child: const _AddRow('Add a field'),
                  ),
                ),
                // When it expires, and when to be reminded (as the web vault's
                // "Expires"): the reminders are the app's and the extension's.
                // A card's own expiration month and year are its date, as in the
                // web vault, so it has no other.
                if (kind != 'card') ...[
                  gap(),
                  TextField(
                    controller: expires,
                    keyboardType: TextInputType.datetime,
                    onChanged: (_) => setState(expiresChanged),
                    decoration: labeledField(
                      context,
                      'Expires (optional)',
                      hint: 'Such as 2027-03-31',
                      suffix: _trailing([
                        if (expires.text.isNotEmpty)
                          IconButton(
                            tooltip: 'No expiration date',
                            icon: Icon(
                              Icons.close_rounded,
                              size: 20,
                              color: c.muted,
                            ),
                            onPressed: () => setState(() {
                              expires.clear();
                              expiresChanged();
                            }),
                          ),
                        IconButton(
                          tooltip: 'Pick a date',
                          icon: Icon(
                            Icons.event_outlined,
                            size: 20,
                            color: c.muted,
                          ),
                          onPressed: pickExpiry,
                        ),
                      ]),
                    ),
                  ),
                ],
                if (kind != 'card' && expires.text.trim().isNotEmpty) ...[
                  gap(),
                  DropdownButtonFormField<String>(
                    key: const ValueKey('remind-me'),
                    initialValue: remind,
                    isExpanded: true,
                    style: _valueStyle,
                    decoration: labeledField(context, 'Remind me'),
                    items: [
                      for (final (value, words) in remindOptions)
                        DropdownMenuItem(value: value, child: Text(words)),
                    ],
                    onChanged: (v) => setState(() => remind = v ?? ''),
                  ),
                ],
                gap(),
                TextField(
                  controller: notes,
                  minLines: 3,
                  maxLines: 8,
                  decoration: labeledField(context, 'Notes'),
                ),
                gap(),
                folderPicker(),
                gap(),
                FieldBox(
                  child: SwitchListTile(
                    value: favorite,
                    onChanged: (v) => setState(() => favorite = v),
                    title: const Text('Favorite'),
                    contentPadding: _switchInset,
                  ),
                ),
                gap(),
                FieldBox(
                  child: SwitchListTile(
                    key: const ValueKey('ask-for-master-password'),
                    value: reprompt,
                    onChanged: (v) => setState(() => reprompt = v),
                    title: const Text('Ask for master password'),
                    subtitle: const Text(
                      'Before its secrets are shown, copied, or filled',
                    ),
                    contentPadding: _switchInset,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
    // Android's Back and Escape close it as Cancel does (close). Escape is
    // the editor's own, wherever the person is in it, a field included: a
    // click outside doesn't close it, so the dialog's own Escape is off too.
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (popped, _) {
        if (!popped) close();
      },
      child: CallbackShortcuts(
        bindings: {
          const SingleActivator(LogicalKeyboardKey.escape): () =>
              Navigator.of(context).maybePop(),
        },
        child: FocusScope(autofocus: true, child: editor),
      ),
    );
  }
}

/// Closing the editor with something changed: true once the person says to
/// discard it, as the web vault asks.
Future<bool> askToDiscard(BuildContext context) async =>
    await showDialog<bool>(
      context: context,
      builder: (context) {
        final scheme = Theme.of(context).colorScheme;
        return AlertDialog(
          title: const Text('Discard your changes?'),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('Keep editing'),
            ),
            FilledButton(
              style: FilledButton.styleFrom(
                backgroundColor: scheme.error,
                foregroundColor: scheme.onError,
              ),
              onPressed: () => Navigator.pop(context, true),
              child: const Text('Discard'),
            ),
          ],
        );
      },
    ) ==
    true;

/// The editor's spacing, on the scale an item's page and Settings use: 16
/// from the screen's edge to a box, 16 more to what's in it, 12 between one
/// field and the next, and a heading over each section as Settings' groups
/// have theirs.
const _gutter = 16.0;
const _between = 12.0;
const _heading = EdgeInsets.fromLTRB(4, 20, 4, 8);

/// A switch row in a box: its title where the fields' text starts.
const _switchInset = EdgeInsets.only(left: 16, right: 12);

/// "Add a website" and "Add a field": one look, starting where the
/// headings do.
class _AddRow extends StatelessWidget {
  const _AddRow(this.label);

  final String label;

  @override
  Widget build(BuildContext context) {
    final primary = Theme.of(context).colorScheme.primary;
    return ConstrainedBox(
      constraints: const BoxConstraints(minHeight: 44),
      child: Padding(
        padding: const EdgeInsets.only(right: 10),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Glyph(Glyphs.plus, size: 18, color: primary),
            const SizedBox(width: 8),
            Text(
              label,
              style: TextStyle(
                color: primary,
                fontSize: 15,
                fontWeight: FontWeight.w600,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// A line under a field or a section, as Settings' groups have theirs.
class _Footnote extends StatelessWidget {
  const _Footnote(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(4, 8, 4, 0),
    child: Text(
      text,
      style: TextStyle(color: context.colors.muted, fontSize: 13, height: 1.35),
    ),
  );
}

/// The kinds of custom field, as the editor offers them.
const _fieldKinds = {0: 'Text', 1: 'Hidden', 2: 'Yes or no'};

/// Upstream's three kinds, as templates name them.
const _kindNames = {0: 'text', 1: 'hidden', 2: 'boolean'};

/// The kinds a template's field can be, as the editor offers them (the web
/// vault's, templates.ts).
const _templateKinds = {
  'text': 'Text',
  'hidden': 'Hidden',
  'boolean': 'Yes or no',
  'date': 'Date',
  'url': 'Website',
  'email': 'Email',
  'phone': 'Phone',
  'number': 'Number',
  'totp': 'One-time code',
};

/// "Remind me": the default (stored as no field), earlier, later, or never,
/// as `keykeeper.remind` holds them (core/src/expiration.rs).
const _remindOptions = [
  ('', '30, 7, and 1 day before'),
  ('90,30,7,1', '90, 30, 7, and 1 day before'),
  ('7,1', '7 and 1 day before'),
  ('none', "Don't remind me"),
];

/// A field's name, asked for: null when canceled.
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
