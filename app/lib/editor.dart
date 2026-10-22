// Adding and editing an item. Editing shows its secrets in the fields, so
// they pass through Dart while the editor is open, as typing them does; the
// core encrypts what's saved (docs/core.md, "Memory").
import 'package:flutter/material.dart';

import 'core.dart';
import 'generator.dart';
import 'theme.dart';
import 'widgets.dart';

/// Opens the editor; resolves to the saved item's id ('' for a new one), or null if cancelled.
Future<String?> openEditor(
  BuildContext context, {
  required Core core,
  required Map<String, dynamic> places,
  String? id,
  Map<String, dynamic>? prefill,
  required bool asDialog,
}) {
  final editor = ItemEditor(
    core: core,
    places: places,
    id: id,
    prefill: prefill,
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

class _Field {
  _Field(String name, String value, this.hidden)
    : name = TextEditingController(text: name),
      value = TextEditingController(text: value);

  final TextEditingController name;
  final TextEditingController value;
  bool hidden;
}

class ItemEditor extends StatefulWidget {
  const ItemEditor({
    super.key,
    required this.core,
    required this.places,
    this.id,
    this.prefill,
  });

  final Core core;
  final Map<String, dynamic> places;
  final String? id;

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
  bool hidden = true;
  String? folderId;
  String? organizationId;
  String? collectionId;
  String? error;
  bool working = false;

  bool get editing => widget.id != null;

  @override
  void initState() {
    super.initState();
    final id = widget.id;
    final item = id == null ? null : widget.core.item(id);
    if (id != null && item != null) {
      final reveal = widget.core.reveal;
      kind = item['kind'] as String;
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
        // Yes-or-no, linked, and nameless fields are the web vault's to
        // edit, and our own data (fill rules, an expiration date) isn't a
        // custom field; saving keeps them (core/src/vault.rs, Kept).
        if (f['editable'] == false) continue;
        final hidden = f['hidden'] == true;
        fields.add(
          _Field(
            f['name'] as String,
            (hidden ? reveal(id, 'field:$i') : f['value'] as String?) ?? '',
            hidden,
          ),
        );
      }
      favorite = item['favorite'] == true;
      folderId = item['folderId'] as String?;
      organizationId = item['organizationId'] as String?;
      final collections =
          (item['collectionIds'] as List?)?.cast<String>() ?? const [];
      collectionId = collections.isEmpty ? null : collections.first;
    }
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

  List<Map<String, dynamic>> get organizations =>
      (widget.places['organizations'] as List).cast<Map<String, dynamic>>();

  List<Map<String, dynamic>> get collections {
    final org = organizations
        .where((o) => o['id'] == organizationId)
        .firstOrNull;
    return ((org?['collections'] as List?) ?? const [])
        .cast<Map<String, dynamic>>();
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
              'value': f.value.text,
              'hidden': f.hidden,
            },
      ],
      'folderId': folderId,
      'organizationId': organizationId,
      'collectionIds': [?collectionId],
      'favorite': favorite,
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
    Row(
      children: [
        Expanded(
          child: DropdownButtonFormField<String?>(
            initialValue: expMonth,
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
        ),
        const SizedBox(width: 12),
        Expanded(
          child: _detail(
            card['expYear']!,
            'Expiration year',
            keyboard: TextInputType.number,
          ),
        ),
      ],
    ),
    gap(),
    _detail(
      card['code']!,
      'Security code',
      secret: true,
      keyboard: TextInputType.number,
    ),
  ];

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

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget gap([double h = 12]) => SizedBox(height: h);
    final folders = (widget.places['folders'] as List)
        .cast<Map<String, dynamic>>();
    return Scaffold(
      appBar: AppBar(
        title: Text(editing ? 'Edit item' : 'New item'),
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
          if (!editing) ...[
            SegmentedButton<String>(
              showSelectedIcon: false,
              segments: const [
                ButtonSegment(value: 'login', label: Text('Login')),
                ButtonSegment(value: 'note', label: Text('Note')),
                ButtonSegment(value: 'card', label: Text('Card')),
                ButtonSegment(value: 'identity', label: Text('Identity')),
              ],
              selected: {kind},
              onSelectionChanged: (s) => setState(() {
                kind = s.first;
                if (kind == 'login' && uris.isEmpty) {
                  uris.add(TextEditingController());
                }
              }),
            ),
            gap(16),
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
            Text(
              'Websites',
              style: TextStyle(
                color: c.muted,
                fontSize: 12,
                fontWeight: FontWeight.w600,
              ),
            ),
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
          gap(),
          TextField(
            controller: notes,
            minLines: 3,
            maxLines: 8,
            decoration: const InputDecoration(labelText: 'Notes'),
          ),
          gap(16),
          Text(
            'Custom fields',
            style: TextStyle(
              color: c.muted,
              fontSize: 12,
              fontWeight: FontWeight.w600,
            ),
          ),
          for (var i = 0; i < fields.length; i++) ...[
            gap(6),
            Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: fields[i].name,
                    decoration: const InputDecoration(hintText: 'Name'),
                  ),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: TextField(
                    controller: fields[i].value,
                    obscureText: fields[i].hidden,
                    decoration: const InputDecoration(hintText: 'Value'),
                  ),
                ),
                IconButton(
                  tooltip: fields[i].hidden
                      ? 'Hidden: make it visible'
                      : 'Visible: hide it',
                  icon: Icon(
                    fields[i].hidden
                        ? Icons.visibility_off_outlined
                        : Icons.visibility_outlined,
                  ),
                  onPressed: () =>
                      setState(() => fields[i].hidden = !fields[i].hidden),
                ),
                IconButton(
                  tooltip: 'Remove',
                  icon: const Icon(Icons.remove_circle_outline),
                  onPressed: () => setState(() => fields.removeAt(i)),
                ),
              ],
            ),
          ],
          Align(
            alignment: Alignment.centerLeft,
            child: TextButton.icon(
              onPressed: () =>
                  setState(() => fields.add(_Field('', '', false))),
              icon: const Icon(Icons.add_rounded),
              label: const Text('Add a field'),
            ),
          ),
          gap(16),
          DropdownButtonFormField<String?>(
            initialValue: folderId,
            decoration: const InputDecoration(labelText: 'Folder'),
            items: [
              const DropdownMenuItem(value: null, child: Text('No folder')),
              for (final f in folders)
                DropdownMenuItem(
                  value: f['id'] as String,
                  child: Text(f['name'] as String),
                ),
            ],
            onChanged: (v) => setState(() => folderId = v),
          ),
          if (organizations.isNotEmpty) ...[
            gap(),
            DropdownButtonFormField<String?>(
              initialValue: organizationId,
              decoration: const InputDecoration(labelText: 'Owner'),
              // Moving an item between owners is for the web vault, which re-encrypts it.
              items: [
                const DropdownMenuItem(value: null, child: Text('My vault')),
                for (final o in organizations)
                  DropdownMenuItem(
                    value: o['id'] as String,
                    child: Text(o['name'] as String),
                  ),
              ],
              onChanged: editing
                  ? null
                  : (v) => setState(() {
                      organizationId = v;
                      final first = collections.firstOrNull;
                      collectionId = first?['id'] as String?;
                    }),
            ),
          ],
          if (organizationId != null) ...[
            gap(),
            DropdownButtonFormField<String?>(
              initialValue: collectionId,
              decoration: const InputDecoration(labelText: 'Collection'),
              items: [
                for (final col in collections)
                  DropdownMenuItem(
                    value: col['id'] as String,
                    child: Text(col['name'] as String),
                  ),
              ],
              onChanged: editing
                  ? null
                  : (v) => setState(() => collectionId = v),
            ),
          ],
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
