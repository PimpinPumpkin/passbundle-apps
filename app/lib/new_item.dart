// New item: what are you saving? As the web vault's picker has it
// (passbundle-clients, new-item-dialog.component.ts): the kinds people save
// most as big tiles, then, under More categories, the templates (built in,
// and the person's, their organizations', and their MSP's), with a search.
// The editor then opens with that kind, or the template's kind and fields.
import 'package:flutter/material.dart';

import 'glyphs.dart';
import 'theme.dart';
import 'widgets.dart';

/// What the person picked: a kind of item, and the template, if any.
class NewItemChoice {
  const NewItemChoice(this.kind, [this.template]);

  /// login, note, card, or identity.
  final String kind;

  /// `{id, name, base, category, fields: [{label, kind, value}]}`, from
  /// the core (core/src/templates.rs).
  final Map<String, dynamic>? template;
}

/// The kinds people save most, as the web vault's picker has them.
const _main = [
  (ItemKind.login, 'Website or app'),
  (ItemKind.note, 'Important text'),
  (ItemKind.card, 'Credit or debit card'),
  (ItemKind.identity, 'Personal info'),
];

/// Asks what's being saved: null if cancelled. On a computer a dialog, on a
/// phone a page of its own.
Future<NewItemChoice?> chooseNewItem(
  BuildContext context, {
  required List<Map<String, dynamic>> templates,
  required bool asDialog,
}) {
  final picker = NewItemPicker(templates: templates, dialog: asDialog);
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
  });

  /// The core's template groups (Core.templates).
  final List<Map<String, dynamic>> templates;
  final bool dialog;

  @override
  State<NewItemPicker> createState() => _NewItemPickerState();
}

class _NewItemPickerState extends State<NewItemPicker> {
  bool more = false;
  String query = '';

  bool matches(String name) =>
      name.toLowerCase().contains(query.trim().toLowerCase());

  void pick(NewItemChoice choice) => Navigator.of(context).pop(choice);

  void pickTemplate(Map<String, dynamic> t) =>
      pick(NewItemChoice(t['base'] == 'login' ? 'login' : 'note', t));

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
    return Scaffold(
      appBar: AppBar(
        title: const Text('What are you saving?'),
        automaticallyImplyLeading: false,
        leading: IconButton(
          tooltip: 'Cancel',
          icon: const Icon(Icons.close_rounded),
          onPressed: () => Navigator.of(context).pop(),
        ),
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(20, 4, 20, 28),
        children: [
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
                  for (var i = 0; i < _main.length; i += perRow) ...[
                    if (i > 0) const SizedBox(height: 10),
                    IntrinsicHeight(
                      child: Row(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          for (var j = i; j < i + perRow; j++) ...[
                            if (j > i) const SizedBox(width: 10),
                            Expanded(
                              child: _MainKind(
                                kind: _main[j].$1,
                                subtitle: _main[j].$2,
                                onTap: () =>
                                    pick(NewItemChoice(_main[j].$1.kind)),
                              ),
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
            if (builtIn.isEmpty && custom.isEmpty)
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
          ],
        ],
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
  });

  final ItemKind kind;
  final String subtitle;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final kindName = kind == ItemKind.note ? 'Secure note' : kind.one;
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

/// Templates, two or three to a row, each with its category's jewel.
class _Templates extends StatelessWidget {
  const _Templates({required this.templates, required this.onPick});

  final List<Map<String, dynamic>> templates;
  final ValueChanged<Map<String, dynamic>> onPick;

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
                            onTap: () => onPick(templates[j]),
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
  const _Template({required this.template, required this.onTap});

  final Map<String, dynamic> template;
  final VoidCallback onTap;

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
                jewel: Categories.of(template['category'] as String?),
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
            ],
          ),
        ),
      ),
    ),
  );
}
