// An item's version history, as the web vault has it (passbundle-clients,
// item-versions.component.ts): its earlier versions, newest first, with when
// each was saved and by whom; one beside the item as it is now, field by
// field, its secrets hidden until shown; restoring one (a normal edit, which
// the core makes, core/src/history.rs); and deleting one, or clearing them
// all, where the person may. Someone whose shared folder hides the item's
// passwords sees when and by whom only, as the server sends it.
import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

/// Opens an item's history: a page on a phone, a dialog on a computer.
/// True if a version was restored, so the item changed.
Future<bool> openHistory(
  BuildContext context, {
  required Core core,
  required String id,
  required String name,
  required bool asDialog,
}) async {
  final view = HistoryView(core: core, id: id, name: name, asDialog: asDialog);
  final restored = asDialog
      ? await showDialog<bool>(
          context: context,
          builder: (context) => Dialog(
            clipBehavior: Clip.antiAlias,
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 720, maxHeight: 760),
              child: view,
            ),
          ),
        )
      : await Navigator.of(context)
            .push<bool>(MaterialPageRoute(builder: (_) => view));
  return restored == true;
}

/// When a version was saved, as people read it: "October 2, 2026, 3:30 PM".
String whenSaved(String? iso) {
  final at = iso == null ? null : DateTime.tryParse(iso)?.toLocal();
  if (at == null) return 'At an unknown time';
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
  final hour = at.hour % 12 == 0 ? 12 : at.hour % 12;
  final minute = at.minute.toString().padLeft(2, '0');
  final half = at.hour < 12 ? 'AM' : 'PM';
  return '${months[at.month - 1]} ${at.day}, ${at.year}, $hour:$minute $half';
}

/// Asks before something that can't be undone: true to go ahead.
Future<bool> _sure(
  BuildContext context, {
  required String title,
  required String content,
  required String action,
}) async =>
    await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(title),
        content: Text(content),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: Text(action),
          ),
        ],
      ),
    ) ==
    true;

class HistoryView extends StatefulWidget {
  const HistoryView({
    super.key,
    required this.core,
    required this.id,
    required this.name,
    required this.asDialog,
  });

  final Core core;
  final String id;
  final String name;
  final bool asDialog;

  @override
  State<HistoryView> createState() => _HistoryViewState();
}

class _HistoryViewState extends State<HistoryView> {
  Map<String, dynamic>? history;
  String? error;
  bool working = false;

  @override
  void initState() {
    super.initState();
    load();
  }

  Future<void> load() async {
    setState(() {
      history = null;
      error = null;
    });
    final answer = await whileBusy(() => Core.history(widget.id));
    if (!mounted) return;
    setState(() {
      if (answer.ok) {
        history = answer.body;
      } else {
        // A server without item history says it hasn't the item's.
        error = answer.signedOut
            ? 'You\'ve been signed out. Sign in again to see its history.'
            : answer.error;
      }
    });
  }

  Future<void> clear() async {
    final go = await _sure(
      context,
      title: 'Clear this item\'s history?',
      content: 'Its earlier versions are deleted for everyone who can open it. The item stays as it is now.',
      action: 'Clear history',
    );
    if (!go || !mounted) return;
    setState(() => working = true);
    final answer = await whileBusy(() => Core.deleteHistory(widget.id));
    if (!mounted) return;
    setState(() => working = false);
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(
          answer.ok ? 'History cleared' : answer.error ?? 'Couldn\'t clear it.',
        ),
      ),
    );
    await load();
  }

  Future<void> open(Map<String, dynamic> version) async {
    final history = this.history!;
    final view = VersionView(
      core: widget.core,
      id: widget.id,
      version: version,
      canDelete: history['canClear'] == true,
    );
    final done = widget.asDialog
        ? await showDialog<String>(
            context: context,
            builder: (context) => Dialog(
              clipBehavior: Clip.antiAlias,
              child: ConstrainedBox(
                constraints: const BoxConstraints(
                  maxWidth: 820,
                  maxHeight: 760,
                ),
                child: view,
              ),
            ),
          )
        : await Navigator.of(context)
              .push<String>(MaterialPageRoute(builder: (_) => view));
    if (!mounted) return;
    if (done == 'restored') {
      Navigator.of(context).pop(true);
    } else if (done == 'deleted') {
      await load();
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final history = this.history;
    final versions = ((history?['versions'] as List?) ?? const [])
        .cast<Map<String, dynamic>>();
    final hidden = history?['hidePasswords'] == true;
    final unreadable = (history?['unreadable'] as num?)?.toInt() ?? 0;
    return Scaffold(
      appBar: AppBar(
        automaticallyImplyLeading: !widget.asDialog,
        leading: widget.asDialog
            ? IconButton(
                tooltip: 'Close',
                icon: const Icon(Icons.close_rounded),
                onPressed: () => Navigator.of(context).pop(false),
              )
            : null,
        title: const Text('Version history'),
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(20, 4, 20, 28),
        children: [
          Text(
            widget.name,
            style: const TextStyle(fontSize: 18, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 6),
          if (history != null)
            Text(
              hidden
                  ? 'You can\'t see this item\'s passwords, so its earlier versions show only when they were saved and by whom.'
                  : 'Everyone who can open this item can see its earlier versions, including old passwords.',
              style: TextStyle(color: c.muted, fontSize: 13, height: 1.35),
            ),
          const SizedBox(height: 16),
          if (error != null)
            Text(error!, style: TextStyle(color: c.muted))
          else if (history == null)
            const Center(
              child: Padding(
                padding: EdgeInsets.all(24),
                child: CircularProgressIndicator(strokeWidth: 2),
              ),
            )
          else if (versions.isEmpty)
            Text('No earlier versions yet.', style: TextStyle(color: c.muted))
          else
            RestingCard(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (var i = 0; i < versions.length; i++) ...[
                    if (i > 0) const Divider(indent: 16),
                    _VersionRow(
                      version: versions[i],
                      onTap: versions[i]['opens'] == true
                          ? () => open(versions[i])
                          : null,
                    ),
                  ],
                ],
              ),
            ),
          if (unreadable > 0) ...[
            const SizedBox(height: 10),
            Text(
              'Earlier versions that can\'t be opened: $unreadable',
              style: TextStyle(color: c.muted, fontSize: 13),
            ),
          ],
          if (history?['canClear'] == true && versions.isNotEmpty) ...[
            const SizedBox(height: 16),
            Align(
              alignment: Alignment.centerLeft,
              child: TextButton(
                style: TextButton.styleFrom(
                  foregroundColor: Theme.of(context).colorScheme.error,
                ),
                onPressed: working ? null : clear,
                child: const Text('Clear history'),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

class _VersionRow extends StatelessWidget {
  const _VersionRow({required this.version, this.onTap});

  final Map<String, dynamic> version;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final changes = (version['changes'] as num?)?.toInt();
    final details = [
      ?version['savedBy'] as String?,
      if (changes != null) changes == 1 ? '1 change' : '$changes changes',
    ].join(' · ');
    return ListTile(
      onTap: onTap,
      title: Text(whenSaved(version['revisionDate'] as String?)),
      subtitle: details.isEmpty ? null : Text(details),
      trailing: onTap == null
          ? null
          : Icon(Icons.chevron_right_rounded, color: c.muted),
    );
  }
}

/// One version beside the item as it is now: every field either has, the
/// ones that differ marked, secrets hidden until shown.
class VersionView extends StatefulWidget {
  const VersionView({
    super.key,
    required this.core,
    required this.id,
    required this.version,
    required this.canDelete,
  });

  final Core core;
  final String id;
  final Map<String, dynamic> version;
  final bool canDelete;

  @override
  State<VersionView> createState() => _VersionViewState();
}

class _VersionViewState extends State<VersionView> {
  late List<Map<String, dynamic>> rows = (widget.version['rows'] as List)
      .cast<Map<String, dynamic>>();
  bool shown = false;
  bool working = false;

  String get versionId => widget.version['id'] as String;
  String get when => whenSaved(widget.version['revisionDate'] as String?);

  void toggle() {
    final answer = widget.core.itemVersion(
      widget.id,
      versionId,
      reveal: !shown,
    );
    if (!answer.ok) return;
    setState(() {
      shown = !shown;
      rows = (answer.body['rows'] as List).cast<Map<String, dynamic>>();
    });
  }

  Future<void> restore() async {
    final go = await _sure(
      context,
      title: 'Restore this version?',
      content:
          'The item goes back to how it was on $when. How it is now stays in its version history, so you can come back to it.',
      action: 'Restore',
    );
    if (!go || !mounted) return;
    setState(() => working = true);
    final answer = await whileBusy(
      () => Core.restoreVersion(widget.id, versionId),
    );
    if (!mounted) return;
    setState(() => working = false);
    if (answer.ok) {
      Navigator.of(context).pop('restored');
      return;
    }
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(answer.error ?? 'Couldn\'t restore it.')),
    );
  }

  Future<void> delete() async {
    final go = await _sure(
      context,
      title: 'Delete this version?',
      content:
          'The version from $when is deleted for everyone who can open this item.',
      action: 'Delete',
    );
    if (!go || !mounted) return;
    setState(() => working = true);
    final answer = await whileBusy(
      () => Core.deleteHistory(widget.id, versionId),
    );
    if (!mounted) return;
    setState(() => working = false);
    if (answer.ok) {
      Navigator.of(context).pop('deleted');
      return;
    }
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(answer.error ?? 'Couldn\'t delete it.')),
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final scheme = Theme.of(context).colorScheme;
    final secrets = rows.any((r) => r['secret'] == true);
    final changes = rows.where((r) => r['changed'] == true).length;
    return Scaffold(
      appBar: AppBar(title: Text('Version from $when')),
      body: LayoutBuilder(
        builder: (context, constraints) {
          // Side by side where there's room; on a phone, each field's two
          // values one over the other.
          final wide = constraints.maxWidth >= 560;
          String value(Map<String, dynamic> r, String which) {
            final v = r[which] as String?;
            if (v == null) return '••••••••';
            return v.isEmpty ? '(empty)' : v;
          }

          Widget cell(String text, {bool strong = false, bool muted = false}) =>
              SelectableText(
                text,
                style: TextStyle(
                  fontSize: 14,
                  fontWeight: strong ? FontWeight.w600 : FontWeight.w400,
                  color: muted ? c.muted : null,
                ),
              );
          Widget changed() => Container(
            margin: const EdgeInsets.only(top: 4),
            padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
            decoration: BoxDecoration(
              color: c.tint,
              borderRadius: BorderRadius.circular(6),
            ),
            child: Text(
              'Changed',
              style: TextStyle(
                color: scheme.primary,
                fontSize: 11,
                fontWeight: FontWeight.w700,
              ),
            ),
          );
          final table = Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (wide)
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 12, 16, 8),
                  child: Row(
                    children: [
                      Expanded(flex: 2, child: cell('Field', muted: true)),
                      Expanded(
                        flex: 3,
                        child: cell('This version', muted: true),
                      ),
                      Expanded(flex: 3, child: cell('Now', muted: true)),
                    ],
                  ),
                ),
              for (var i = 0; i < rows.length; i++) ...[
                if (i > 0 || wide) const Divider(indent: 16),
                Container(
                  color: rows[i]['changed'] == true
                      ? c.tint.withValues(alpha: 0.5)
                      : null,
                  padding: const EdgeInsets.fromLTRB(16, 10, 16, 10),
                  child: wide
                      ? Row(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Expanded(
                              flex: 2,
                              child: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  cell(
                                    rows[i]['label'] as String,
                                    strong: true,
                                  ),
                                  if (rows[i]['changed'] == true) changed(),
                                ],
                              ),
                            ),
                            Expanded(
                              flex: 3,
                              child: cell(value(rows[i], 'version')),
                            ),
                            Expanded(
                              flex: 3,
                              child: cell(value(rows[i], 'current')),
                            ),
                          ],
                        )
                      : Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Row(
                              children: [
                                Expanded(
                                  child: cell(
                                    rows[i]['label'] as String,
                                    strong: true,
                                  ),
                                ),
                                if (rows[i]['changed'] == true) changed(),
                              ],
                            ),
                            const SizedBox(height: 4),
                            cell('This version: ${value(rows[i], 'version')}'),
                            cell(
                              'Now: ${value(rows[i], 'current')}',
                              muted: true,
                            ),
                          ],
                        ),
                ),
              ],
            ],
          );
          return ListView(
            padding: const EdgeInsets.fromLTRB(20, 4, 20, 28),
            children: [
              Text(
                [
                  ?widget.version['savedBy'] as String?,
                  changes == 0
                      ? 'It looks the same as the item does now.'
                      : changes == 1
                      ? '1 field differs from now.'
                      : '$changes fields differ from now.',
                ].join(' · '),
                style: TextStyle(color: c.muted, fontSize: 13),
              ),
              const SizedBox(height: 12),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  if (widget.version['restorable'] == true)
                    WorkingButton(
                      label: 'Restore this version',
                      working: working,
                      onPressed: restore,
                    ),
                  if (secrets)
                    OutlinedButton(
                      onPressed: toggle,
                      child: Text(
                        shown ? 'Hide hidden values' : 'Show hidden values',
                      ),
                    ),
                  if (widget.canDelete)
                    TextButton(
                      style: TextButton.styleFrom(
                        foregroundColor: scheme.error,
                      ),
                      onPressed: working ? null : delete,
                      child: const Text('Delete this version'),
                    ),
                ],
              ),
              const SizedBox(height: 16),
              RestingCard(child: table),
            ],
          );
        },
      ),
    );
  }
}
