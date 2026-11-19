// New item: what are you saving? As the web vault's picker has it
// (passbundle-clients, new-item-dialog.component.ts): the kinds people save
// most as big tiles, then, under More categories, the templates (built in,
// and the person's, their organizations', and their MSP's), with a search,
// and Create template. The editor then opens with that kind, the
// template's kind and fields, or a new template.
import 'package:flutter/material.dart';

import 'glyphs.dart';
import 'theme.dart';
import 'widgets.dart';

/// What the person picked: a kind of item, and the template, if any; or a
/// new template ("Create template", "Save as template").
class NewItemChoice {
  const NewItemChoice(
    this.kind, [
    this.template,
    this.newTemplate,
    this.fields = const [],
    this.name,
  ]);

  /// A new custom template whose new items are logins or notes (`base`):
  /// a note, starting with [fields] (`{label, kind, expires}`, never values)
  /// and [name], from "Save as template".
  const NewItemChoice.template(
    String base, {
    List<Map<String, dynamic>> fields = const [],
    String? name,
  }) : this('note', null, base, fields, name);

  /// login, note, card, or identity.
  final String kind;

  /// `{id, name, base, category, fields: [{label, kind, value, expires}]}`,
  /// from the core (core/src/templates.rs).
  final Map<String, dynamic>? template;

  /// For a new template: "login" or "note", what its new items will be.
  final String? newTemplate;

  /// A new template's starting fields: `{label, kind, expires}`.
  final List<Map<String, dynamic>> fields;

  /// A new template's starting name.
  final String? name;
}

/// The kinds people save most, as the web vault's picker has them.
const _main = [
  (ItemKind.login, 'Website or app'),
  (ItemKind.note, 'Important text'),
  (ItemKind.card, 'Credit or debit card'),
  (ItemKind.identity, 'Personal info'),
];

/// The kinds the web vault makes and the apps don't yet, which the picker
/// names so nobody looks for them in vain.
const _webVaultKinds = [
  ItemKind.sshKey,
  ItemKind.bankAccount,
  ItemKind.driversLicense,
  ItemKind.passport,
];

/// Asks what's being saved: null if canceled. On a computer a dialog, as
/// tall as what's in it; on a phone a page of its own. [onWebVault] opens
/// the web vault, for the kinds only it makes. [restricted] are the kinds
/// not to offer (Core.restrictedKinds).
Future<NewItemChoice?> chooseNewItem(
  BuildContext context, {
  required List<Map<String, dynamic>> templates,
  required bool asDialog,
  VoidCallback? onWebVault,
  Set<String> restricted = const {},
}) {
  final picker = NewItemPicker(
    templates: templates,
    dialog: asDialog,
    onWebVault: onWebVault,
    restricted: restricted,
  );
  if (asDialog) {
    return showDialog<NewItemChoice>(
      context: context,
      builder: (context) => Dialog(
        clipBehavior: Clip.antiAlias,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 680, maxHeight: 760),
          child: picker,
        ),
      ),
    );
  }
  return Navigator.of(context).push<NewItemChoice>(
    MaterialPageRoute(fullscreenDialog: true, builder: (_) => picker),
  );
}

class NewItemPicker extends StatefulWidget {
  const NewItemPicker({
    super.key,
    required this.templates,
    this.dialog = false,
    this.onWebVault,
    this.restricted = const {},
  });

  /// The core's template groups (Core.templates).
  final List<Map<String, dynamic>> templates;
  final bool dialog;

  /// Opens the web vault, which makes the kinds the apps don't yet.
  final VoidCallback? onWebVault;

  /// The kinds an organization's "Remove card item type" policy removes,
  /// which aren't offered (Core.restrictedKinds).
  final Set<String> restricted;

  @override
  State<NewItemPicker> createState() => _NewItemPickerState();
}

class _NewItemPickerState extends State<NewItemPicker> {
  bool more = false;
  String query = '';

  /// Choosing what a new template's items will be.
  bool creating = false;

  bool matches(String name) =>
      name.toLowerCase().contains(query.trim().toLowerCase());

  void pick(NewItemChoice choice) => Navigator.of(context).pop(choice);

  void pickTemplate(Map<String, dynamic> t) =>
      pick(NewItemChoice(t['base'] == 'login' ? 'login' : 'note', t));

  /// "Create template": what its new items will be, logins or notes, as the
  /// web vault asks (template-chooser.component.ts).
  List<Widget> get creatingChildren {
    final c = context.colors;
    Widget base(String value, ItemKind kind, String label) => Padding(
      padding: const EdgeInsets.only(bottom: 10),
      child: _MainKind(
        kind: kind,
        title: kind.many,
        subtitle: label,
        onTap: () => pick(NewItemChoice.template(value)),
      ),
    );
    return [
      Text(
        'New items from this template will be:',
        style: TextStyle(color: c.muted, height: 1.35),
      ),
      const SizedBox(height: 12),
      if (!widget.restricted.contains('login'))
        base('login', ItemKind.login, 'A username, password, and website'),
      if (!widget.restricted.contains('note'))
        base('note', ItemKind.note, 'Text and custom fields'),
      Padding(
        padding: const EdgeInsets.fromLTRB(4, 4, 4, 12),
        child: Text(
          'Next, name the template and add the fields new items should start with. Values you enter become defaults.',
          style: TextStyle(color: c.muted, fontSize: 13, height: 1.35),
        ),
      ),
      Align(
        alignment: Alignment.centerLeft,
        child: OutlinedButton(
          onPressed: () => setState(() => creating = false),
          child: const Text('Back'),
        ),
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final groups = widget.templates;
    final builtIn =
        groups
            .where((g) => g['name'] == 'Built-in')
            .expand(
              (g) => (g['templates'] as List).cast<Map<String, dynamic>>(),
            )
            .where((t) => matches(t['name'] as String))
            .toList()
          ..sort(
            (a, b) => (a['name'] as String).toLowerCase().compareTo(
              (b['name'] as String).toLowerCase(),
            ),
          );
    final custom = [
      for (final g in groups.where((g) => g['name'] != 'Built-in'))
        (
          g['name'] as String,
          (g['templates'] as List)
              .cast<Map<String, dynamic>>()
              .where((t) => matches(t['name'] as String))
              .toList(),
        ),
    ].where((g) => g.$2.isNotEmpty).toList();
    // None of a kind an organization's policy removes.
    final mainKinds = _main
        .where((m) => !widget.restricted.contains(m.$1.kind))
        .toList();
    // The kinds only the web vault makes, under the rest.
    final elsewhere = _webVaultKinds
        .where((k) => !widget.restricted.contains(k.kind))
        .where((k) => matches(k.one) || matches(k.many))
        .toList();
    final children = [
          // Four in a row where there's room, else two by two.
          LayoutBuilder(
            builder: (context, constraints) {
              final perRow =
                  constraints.maxWidth /
                          MediaQuery.textScalerOf(context).scale(1) >=
                      560
                  ? 4
                  : 2;
              return Column(
                children: [
                  for (var i = 0; i < mainKinds.length; i += perRow) ...[
                    if (i > 0) const SizedBox(height: 10),
                    IntrinsicHeight(
                      child: Row(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          for (var j = i; j < i + perRow; j++) ...[
                            if (j > i) const SizedBox(width: 10),
                            Expanded(
                              child: j < mainKinds.length
                                  ? _MainKind(
                                      kind: mainKinds[j].$1,
                                      subtitle: mainKinds[j].$2,
                                      onTap: () =>
                                          pick(NewItemChoice(mainKinds[j].$1.kind)),
                                    )
                                  : const SizedBox.shrink(),
                            ),
                          ],
                        ],
                      ),
                    ),
                  ],
                ],
              );
            },
          ),
          const SizedBox(height: 16),
          Align(
            alignment: Alignment.centerLeft,
            child: OutlinedButton.icon(
              onPressed: () => setState(() => more = !more),
              icon: Icon(
                more
                    ? Icons.keyboard_arrow_up_rounded
                    : Icons.keyboard_arrow_down_rounded,
              ),
              label: Text(more ? 'Fewer categories' : 'More categories'),
            ),
          ),
          if (more) ...[
            const SizedBox(height: 14),
            TextField(
              autofocus: widget.dialog,
              onChanged: (q) => setState(() => query = q),
              decoration: InputDecoration(
                hintText: 'Find a category',
                prefixIcon: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 12),
                  child: Glyph(Glyphs.search, size: 18, color: c.muted),
                ),
                prefixIconConstraints: const BoxConstraints(minWidth: 42),
              ),
            ),
            const SizedBox(height: 10),
            if (builtIn.isEmpty && custom.isEmpty && elsewhere.isEmpty)
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 12),
                child: Text(
                  'No category matches that.',
                  style: TextStyle(color: c.muted),
                ),
              ),
            _Templates(templates: builtIn, onPick: pickTemplate),
            for (final (name, templates) in custom) ...[
              SectionHeading(
                name,
                padding: const EdgeInsets.fromLTRB(4, 18, 4, 8),
              ),
              _Templates(templates: templates, onPick: pickTemplate),
            ],
            // A new template of the person's own (DESIGN.md, "Making a
            // template in the apps").
            if (query.trim().isEmpty)
              Padding(
                padding: const EdgeInsets.fromLTRB(0, 16, 0, 4),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: OutlinedButton.icon(
                    key: const ValueKey('create-template'),
                    onPressed: () => setState(() => creating = true),
                    icon: const Icon(Icons.add_rounded),
                    label: const Text('Create template'),
                  ),
                ),
              ),
            if (elsewhere.isNotEmpty) ...[
              const SectionHeading(
                'In the web vault',
                padding: EdgeInsets.fromLTRB(4, 18, 4, 4),
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(4, 0, 4, 8),
                child: Text(
                  'PassBundle makes these in the web vault, for now. They show here once they\'re saved.',
                  style: TextStyle(color: c.muted, fontSize: 13, height: 1.35),
                ),
              ),
              _Templates(
                templates: [
                  for (final k in elsewhere)
                    {'name': k.one, 'kind': k.kind},
                ],
                onPick: widget.onWebVault == null
                    ? null
                    : (_) => widget.onWebVault!(),
                trailing: widget.onWebVault == null
                    ? null
                    : Icon(Icons.open_in_new_rounded, size: 16, color: c.muted),
              ),
            ],
          ],
    ];
    final title = creating ? 'New template' : 'What are you saving?';
    final shown = creating ? creatingChildren : children;
    if (widget.dialog) {
      return DialogPage(
        title: title,
        closeTooltip: 'Cancel',
        children: shown,
      );
    }
    return Scaffold(
      appBar: AppBar(
        title: Text(title),
        automaticallyImplyLeading: false,
        leading: IconButton(
          tooltip: 'Cancel',
          icon: const Icon(Icons.close_rounded),
          onPressed: () => Navigator.of(context).pop(),
        ),
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(20, 4, 20, 28),
        children: shown,
      ),
    );
  }
}

/// One of the main kinds, as a big tile: its jewel, its name, and what it's for.
class _MainKind extends StatelessWidget {
  const _MainKind({
    required this.kind,
    required this.subtitle,
    required this.onTap,
    this.title,
  });

  final ItemKind kind;
  final String subtitle;
  final VoidCallback onTap;

  /// What it's called, when not the kind's own name: "Logins" for what a
  /// new template's items will be.
  final String? title;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final kindName = title ?? kind.one;
    return RestingCard(
      radius: 14,
      child: InkWell(
        onTap: onTap,
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 112),
          child: Padding(
            padding: const EdgeInsets.all(14),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                JewelTile(jewel: kind, size: 36),
                const SizedBox(height: 10),
                Text(
                  kindName,
                  style: const TextStyle(
                    fontSize: 15,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                const SizedBox(height: 2),
                Text(subtitle, style: TextStyle(color: c.muted, fontSize: 13)),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Templates, two or three to a row, each with its category's jewel (or,
/// for a kind the web vault makes, its kind's, with where it opens).
class _Templates extends StatelessWidget {
  const _Templates({required this.templates, required this.onPick, this.trailing});

  final List<Map<String, dynamic>> templates;
  final ValueChanged<Map<String, dynamic>>? onPick;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final scale = MediaQuery.textScalerOf(context).scale(1);
      final perRow = constraints.maxWidth / scale >= 560
          ? 3
          : constraints.maxWidth / scale >= 330
          ? 2
          : 1;
      return Column(
        children: [
          for (var i = 0; i < templates.length; i += perRow) ...[
            if (i > 0) const SizedBox(height: 8),
            Row(
              children: [
                for (var j = i; j < i + perRow; j++) ...[
                  if (j > i) const SizedBox(width: 8),
                  Expanded(
                    child: j < templates.length
                        ? _Template(
                            template: templates[j],
                            trailing: trailing,
                            onTap: onPick == null
                                ? null
                                : () => onPick!(templates[j]),
                          )
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
}

class _Template extends StatelessWidget {
  const _Template({required this.template, required this.onTap, this.trailing});

  final Map<String, dynamic> template;
  final VoidCallback? onTap;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) => RestingCard(
    radius: 12,
    child: InkWell(
      onTap: onTap,
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 52),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          child: Row(
            children: [
              JewelTile(
                jewel: switch (template['kind']) {
                  final String kind => ItemKind.of(kind),
                  _ => Categories.forItem(
                    template['category'] as String?,
                    template['id'] as String?,
                  ),
                },
                size: 28,
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  template['name'] as String,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(fontWeight: FontWeight.w600),
                ),
              ),
              ?trailing,
            ],
          ),
        ),
      ),
    ),
  );
}
