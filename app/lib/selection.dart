// Several items selected in a list, and one thing done to all of them
// (docs/core.md, "Selecting several items"): favorites, a folder or shared
// folder, the trash. The lists keep which items are selected (vault.dart);
// here is what's done with them: the bar with the actions, where to move
// them, the question asked first, and the words for what happened.
//
// What a person may do with each item is the core's to say (`Core.bulkPlan`
// before anything is sent, `Core.bulk` after): nothing here decides it. The
// items left as they are get said with why, before and after.
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'folder_looks.dart';
import 'glyphs.dart';
import 'places.dart';
import 'theme.dart';
import 'widgets.dart';

/// "1 item", "3 items".
String items(int n) => n == 1 ? '1 item' : '$n items';

/// Whether the key that selects without opening is down: Command on a Mac,
/// Ctrl elsewhere.
bool commandHeld() {
  final keys = HardwareKeyboard.instance;
  return switch (defaultTargetPlatform) {
    TargetPlatform.macOS || TargetPlatform.iOS => keys.isMetaPressed,
    _ => keys.isControlPressed,
  };
}

enum BulkKind { favorite, unfavorite, trash, folder, sharedFolder }

/// One thing to do to every selected item.
class BulkAction {
  const BulkAction.favorite(bool on)
    : kind = on ? BulkKind.favorite : BulkKind.unfavorite,
      to = null,
      toName = null,
      from = null,
      owner = null;

  const BulkAction.trash()
    : kind = BulkKind.trash,
      to = null,
      toName = null,
      from = null,
      owner = null;

  /// Into one of the person's folders, or none ([to] null).
  const BulkAction.folder(this.to, this.toName)
    : kind = BulkKind.folder,
      from = null,
      owner = null;

  /// Into an organization's shared folder, out of the one shown ([from]).
  const BulkAction.sharedFolder(
    String this.to,
    this.toName, {
    this.from,
    this.owner,
  }) : kind = BulkKind.sharedFolder;

  final BulkKind kind;

  /// The folder or shared folder they go to, and its name.
  final String? to;
  final String? toName;

  /// The shared folder they're shown in, which they leave.
  final String? from;

  /// The organization a shared folder is in, by name.
  final String? owner;

  /// As the core reads it (core/src/app/bulk.rs).
  Map<String, dynamic> get json => switch (kind) {
    BulkKind.favorite => {'do': 'favorite', 'favorite': true},
    BulkKind.unfavorite => {'do': 'favorite', 'favorite': false},
    BulkKind.trash => {'do': 'trash'},
    BulkKind.folder => {'do': 'folder', 'folderId': to},
    BulkKind.sharedFolder => {'do': 'sharedFolder', 'to': to, 'from': from},
  };

  /// On its button.
  String get button => switch (kind) {
    BulkKind.favorite => 'Add to favorites',
    BulkKind.unfavorite => 'Remove from favorites',
    BulkKind.trash => 'Move to trash',
    BulkKind.folder || BulkKind.sharedFolder => 'Move',
  };

  String get _verb => switch (kind) {
    BulkKind.favorite => 'Add',
    BulkKind.unfavorite => 'Remove',
    _ => 'Move',
  };

  String get _past => switch (kind) {
    BulkKind.favorite => 'added',
    BulkKind.unfavorite => 'removed',
    _ => 'moved',
  };

  String get _where => switch (kind) {
    BulkKind.favorite => 'to Favorites',
    BulkKind.unfavorite => 'from Favorites',
    BulkKind.trash => 'to the trash',
    BulkKind.folder when to == null => 'to Personal, in no folder',
    BulkKind.folder || BulkKind.sharedFolder => 'to $toName',
  };

  /// "Move 3 items to the trash?"
  String ask(int n) => '$_verb ${items(n)} $_where?';

  /// "Moved 3 items to the trash."
  String did(int n) =>
      '${_past[0].toUpperCase()}${_past.substring(1)} ${items(n)} $_where.';

  /// "None of these 3 items can be moved to the trash."
  String cant(int n) => n == 1
      ? 'This item can\'t be $_past $_where'
      : 'None of these $n items can be $_past $_where';

  /// "2 items weren't moved to the trash."
  String didnt(int n) => n == 1
      ? '1 item wasn\'t $_past $_where'
      : '$n items weren\'t $_past $_where';

  /// Why items are left as they are, by the core's name for it
  /// (core/src/app/bulk.rs, `Skip`), after "left as it is:".
  String why(String reason, int n) {
    final one = n == 1;
    final it = one ? 'it' : 'them';
    return switch (reason) {
      'masterPassword' =>
        one
            ? 'it asks for your master password'
            : 'they ask for your master password',
      'noDelete' => 'your organization doesn\'t let you delete $it',
      'shared' =>
        one
            ? 'it belongs to an organization, and stays in its shared folders'
            : 'they belong to an organization, and stay in its shared folders',
      'otherOwner' =>
        '${one ? 'it isn\'t' : 'they aren\'t'} ${owner ?? 'that organization'}\'s, '
            'and moving an item to another owner is done in the web vault',
      'hiddenPasswords' =>
        'your organization hides ${one ? 'its password' : 'their passwords'} from you',
      'noMove' =>
        'your organization doesn\'t let you move $it to another shared folder',
      _ =>
        one
            ? 'it isn\'t in your vault anymore'
            : 'they aren\'t in your vault anymore',
    };
  }

  /// "2 items will be left as they are: ...", before; "2 were left as they
  /// are: ...", after.
  String left(String reason, int n, {bool after = false}) {
    final as = n == 1 ? 'as it is' : 'as they are';
    final start = after
        ? '$n ${n == 1 ? 'was' : 'were'} left $as'
        : '${items(n)} will be left $as';
    return '$start: ${why(reason, n)}.';
  }
}

/// How many of the items an answer of the core's leaves as they are, by
/// why, in the order it gives them.
Map<String, int> leftBy(Answer answer) {
  final by = <String, int>{};
  for (final s in ((answer.body['skipped'] as List?) ?? const [])) {
    by.update((s as Map)['why'] as String, (n) => n + 1, ifAbsent: () => 1);
  }
  return by;
}

List<String> _ids(Answer answer, String name) =>
    ((answer.body[name] as List?) ?? const []).cast<String>();

/// What a bulk action did (`Core.bulk`'s answer), in words: what went, what
/// was left and why, and what didn't go.
class BulkOutcome {
  BulkOutcome(this.action, Answer answer)
    : done = _ids(answer, 'done'),
      failed = _ids(answer, 'failed'),
      left = leftBy(answer),
      error = answer.error,
      synced = answer.body['synced'] != false;

  final BulkAction action;
  final List<String> done;
  final List<String> failed;
  final Map<String, int> left;

  /// Why, for [failed], in the core's words.
  final String? error;

  /// The sync after it came through, so the lists show it.
  final bool synced;

  /// What went and what was left.
  String get said => [
    if (done.isNotEmpty)
      action.did(done.length)
    else if (failed.isEmpty)
      'Nothing was changed.',
    for (final MapEntry(key: why, value: n) in left.entries)
      action.left(why, n, after: true),
    if (!synced && done.isNotEmpty)
      'Couldn\'t sync after that, so the list shows '
          '${done.length == 1 ? 'it as it was' : 'them as they were'} '
          'until the next sync.',
  ].join(' ');
}

/// A selected item's checkbox, in its tile's place.
class SelectBox extends StatelessWidget {
  const SelectBox({
    super.key,
    required this.checked,
    required this.name,
    this.onChanged,
    this.size = 40,
  });

  final bool checked;

  /// The item's name, for a screen reader.
  final String name;

  /// Without it the row takes the tap (a phone's rows).
  final VoidCallback? onChanged;
  final double size;

  @override
  Widget build(BuildContext context) {
    final box = SizedBox.square(
      dimension: size,
      child: Center(
        child: Checkbox(
          value: checked,
          onChanged: (_) => onChanged?.call(),
          materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
          visualDensity: VisualDensity.standard,
          semanticLabel: 'Select $name',
          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(5)),
        ),
      ),
    );
    return onChanged == null
        ? IgnorePointer(child: ExcludeSemantics(child: box))
        : box;
  }
}

/// A list's header on a computer while items are selected: how many, and
/// what can be done with them. An action the person may not do with any of
/// them isn't offered ([onMove] or [onTrash] null).
class SelectionBar extends StatelessWidget {
  const SelectionBar({
    super.key,
    required this.count,
    required this.allFavorites,
    required this.onClear,
    required this.onSelectAll,
    required this.onFavorite,
    required this.onMove,
    required this.onTrash,
  });

  final int count;

  /// Every selected item is a favorite: the star takes them out.
  final bool allFavorites;
  final VoidCallback onClear;
  final VoidCallback onSelectAll;
  final ValueChanged<bool> onFavorite;
  final VoidCallback? onMove;
  final VoidCallback? onTrash;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget button(String tooltip, Widget icon, VoidCallback onPressed) =>
        IconButton(
          tooltip: tooltip,
          onPressed: onPressed,
          style: IconButton.styleFrom(
            minimumSize: const Size.square(40),
            fixedSize: const Size.square(40),
            padding: EdgeInsets.zero,
            tapTargetSize: MaterialTapTargetSize.shrinkWrap,
          ),
          icon: icon,
        );
    return Padding(
      padding: const EdgeInsets.fromLTRB(10, 4, 10, 0),
      child: LayoutBuilder(
        builder: (context, box) => Row(
          children: [
            button(
              'Clear the selection',
              const Icon(Icons.close_rounded, size: 22),
              onClear,
            ),
            const SizedBox(width: 2),
            Expanded(
              child: Text(
                // The narrowest window has room for the number alone.
                box.maxWidth < 300 ? '$count' : '$count selected',
                semanticsLabel: '$count selected',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(
                  fontSize: 15,
                  fontWeight: FontWeight.w700,
                ),
              ),
            ),
            button(
              'Select all',
              const Icon(Icons.select_all_rounded, size: 22),
              onSelectAll,
            ),
            button(
              allFavorites ? 'Remove from favorites' : 'Add to favorites',
              Glyph(
                Glyphs.star,
                size: 20,
                color: allFavorites ? favoriteEdge : c.muted,
                fill: allFavorites ? favoriteColor : null,
                strokeWidth: 1.6,
              ),
              () => onFavorite(!allFavorites),
            ),
            if (onMove != null)
              button('Move', Glyph(Glyphs.folder, size: 20), onMove!),
            if (onTrash != null)
              button(
                'Move to trash',
                const Icon(Icons.delete_outline_rounded, size: 22),
                onTrash!,
              ),
          ],
        ),
      ),
    );
  }
}

/// A phone's bar at the bottom while items are selected, where the tabs
/// are otherwise: what can be done with them.
class PhoneSelectionBar extends StatelessWidget {
  const PhoneSelectionBar({
    super.key,
    required this.allFavorites,
    required this.onFavorite,
    required this.onMove,
    required this.onTrash,
  });

  final bool allFavorites;
  final ValueChanged<bool> onFavorite;
  final VoidCallback? onMove;
  final VoidCallback? onTrash;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget action(String label, Widget icon, VoidCallback onPressed) =>
        Expanded(
          child: InkWell(
            onTap: onPressed,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 10),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  icon,
                  const SizedBox(height: 6),
                  Text(
                    label,
                    maxLines: 2,
                    textAlign: TextAlign.center,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 12,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
    return Material(
      color: c.pane,
      child: SafeArea(
        top: false,
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 68),
          child: IconTheme.merge(
            data: const IconThemeData(size: 22),
            child: Row(
              children: [
                action(
                  allFavorites ? 'Remove from favorites' : 'Add to favorites',
                  Glyph(
                    Glyphs.star,
                    size: 22,
                    color: allFavorites ? favoriteEdge : null,
                    fill: allFavorites ? favoriteColor : null,
                    strokeWidth: 1.6,
                  ),
                  () => onFavorite(!allFavorites),
                ),
                if (onMove != null)
                  action('Move', const Glyph(Glyphs.folder, size: 22), onMove!),
                if (onTrash != null)
                  action(
                    'Move to trash',
                    const Icon(Icons.delete_outline_rounded),
                    onTrash!,
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Where to move the selected items: one of the person's folders (or none)
/// when any is their own ([personal]), or a shared folder of an
/// organization that has any of them ([organizations], by id), of those
/// the person may add items to. Null if dismissed. The place shown now
/// ([here], or No folder with [inNoFolder]) isn't offered.
Future<BulkAction?> chooseWhereToMove(
  BuildContext context, {
  required Places places,
  required int count,
  required bool personal,
  required Set<String> organizations,
  String? here,
  bool inNoFolder = false,
}) => showDialog<BulkAction>(
  context: context,
  builder: (context) {
    final c = context.colors;
    Widget heading(String glyph, String name) => Padding(
      padding: const EdgeInsets.fromLTRB(24, 12, 24, 4),
      child: Row(
        children: [
          Glyph(glyph, size: 18, color: c.muted),
          const SizedBox(width: 10),
          Expanded(
            child: Text(
              name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: c.muted,
                fontSize: 13,
                fontWeight: FontWeight.w700,
              ),
            ),
          ),
        ],
      ),
    );
    Widget note(String text) => Padding(
      padding: const EdgeInsets.fromLTRB(24, 6, 24, 8),
      child: Text(text, style: TextStyle(color: c.muted, fontSize: 14)),
    );
    Widget place(Widget icon, String name, int depth, BulkAction action) =>
        InkWell(
          onTap: () => Navigator.of(context).pop(action),
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 48),
            child: Padding(
              padding: EdgeInsets.fromLTRB(24 + 16.0 * depth, 8, 24, 8),
              child: Row(
                children: [
                  icon,
                  const SizedBox(width: 12),
                  Expanded(
                    child: Text(
                      name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontSize: 15),
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
    final folders = [
      for (final f in places.folders)
        if (f['id'] != here)
          place(
            const Glyph(Glyphs.folder, size: 20),
            f['label'] as String,
            f['depth'] as int,
            BulkAction.folder(f['id'] as String, f['name'] as String),
          ),
      if (!inNoFolder && places.folders.isNotEmpty)
        place(
          Glyph(Glyphs.folder, size: 20, color: c.muted),
          'No folder',
          0,
          const BulkAction.folder(null, null),
        ),
    ];
    return AlertDialog(
      title: Text('Move ${items(count)} to'),
      contentPadding: const EdgeInsets.fromLTRB(0, 8, 0, 8),
      content: SizedBox(
        width: 380,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (personal) ...[
                heading(Glyphs.person, 'Personal'),
                if (places.folders.isEmpty)
                  note('You have no folders yet.')
                else
                  ...folders,
              ],
              for (final o in places.organizations)
                if (organizations.contains(o['id'])) ...[
                  heading(Glyphs.building, o['name'] as String),
                  ...() {
                    final open = places
                        .collectionsOf(o['id'] as String)
                        .where((f) => f['canCreate'] == true);
                    final offered = [
                      for (final f in open)
                        if (f['id'] != here)
                          place(
                            switch (FolderLooks.jewel(f['look'])) {
                              final jewel? => JewelTile(jewel: jewel, size: 22),
                              null => const Glyph(
                                Glyphs.sharedFolder,
                                size: 20,
                              ),
                            },
                            f['label'] as String,
                            f['depth'] as int,
                            BulkAction.sharedFolder(
                              f['id'] as String,
                              f['name'] as String,
                              from: here,
                              owner: o['name'] as String,
                            ),
                          ),
                    ];
                    if (offered.isNotEmpty) return offered;
                    return [
                      note(
                        open.isEmpty
                            ? 'Your organization doesn\'t let you move items '
                                  'to another shared folder.'
                            : 'There\'s no other shared folder to move to.',
                      ),
                    ];
                  }(),
                ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
      ],
    );
  },
);

/// Asks before doing [action] to the selected items ([ids]), saying how
/// many it's for and how many are left as they are and why ([plan], from
/// `Core.bulkPlan`), then does it. Items that ask for the master password
/// are included once it's given here, one time for all of them. The core's
/// answer once it's done, or null if the person didn't go on.
Future<Answer?> confirmBulk(
  BuildContext context, {
  required BulkAction action,
  required List<String> ids,
  required Answer plan,
}) => showDialog<Answer>(
  context: context,
  builder: (_) => _BulkDialog(action: action, ids: ids, plan: plan),
);

class _BulkDialog extends StatefulWidget {
  const _BulkDialog({
    required this.action,
    required this.ids,
    required this.plan,
  });

  final BulkAction action;
  final List<String> ids;
  final Answer plan;

  @override
  State<_BulkDialog> createState() => _BulkDialogState();
}

class _BulkDialogState extends State<_BulkDialog> {
  final password = TextEditingController();
  String? error;
  bool working = false;

  /// The items it's done for whatever is typed here.
  late final sure = ((widget.plan.body['go'] as List?) ?? const []).length;

  /// The items left, by why; those that ask for the master password apart.
  late final left = leftBy(widget.plan)..remove('masterPassword');
  late final asking = leftBy(widget.plan)['masterPassword'] ?? 0;

  @override
  void dispose() {
    password.dispose();
    super.dispose();
  }

  Future<void> go() async {
    final given = password.text;
    // With only items that ask for it, there's nothing to do without it.
    if (working || (sure == 0 && given.isEmpty)) return;
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(
      () => Core.bulk(widget.ids, widget.action.json, password: given),
    );
    if (!mounted) return;
    password.clear();
    // A wrong master password does nothing, and is said under its field.
    if (!answer.ok &&
        given.isNotEmpty &&
        !answer.signedOut &&
        !answer.offline) {
      setState(() {
        working = false;
        error = answer.error;
      });
      return;
    }
    Navigator.of(context).pop(answer);
  }

  @override
  Widget build(BuildContext context) {
    final action = widget.action;
    final nothing = sure == 0 && asking == 0;
    final count = sure > 0 ? sure : asking;
    final field = TextField(
      controller: password,
      obscureText: true,
      autofocus: sure == 0,
      decoration: InputDecoration(
        labelText: 'Master password',
        errorText: error,
      ),
      onSubmitted: (_) => go(),
    );
    final lines = <Widget>[
      if (nothing && left.length == 1)
        () {
          final why = action.why(left.keys.single, left.values.single);
          return Text('${why[0].toUpperCase()}${why.substring(1)}.');
        }()
      else
        for (final MapEntry(key: why, value: n) in left.entries)
          Text(action.left(why, n)),
      if (asking > 0 && sure > 0) ...[
        Text(
          asking == 1
              ? '1 more item asks for your master password. Enter it to '
                    'include that item, or leave it empty to leave the item '
                    'as it is.'
              : '$asking more items ask for your master password. Enter it '
                    'to include them, or leave it empty to leave them as '
                    'they are.',
        ),
        field,
      ] else if (asking > 0) ...[
        Text(
          asking == 1
              ? 'This item asks for your master password.'
              : 'These items ask for your master password.',
        ),
        field,
      ],
      if (action.kind == BulkKind.trash && !nothing)
        Text(
          count == 1 && asking == 0
              ? 'You can restore it from the trash in the web vault.'
              : 'You can restore them from the trash in the web vault.',
        ),
    ];
    return AlertDialog(
      title: Text(nothing ? action.cant(widget.ids.length) : action.ask(count)),
      content: lines.isEmpty
          ? null
          : SizedBox(
              width: 380,
              child: SingleChildScrollView(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    for (final (i, line) in lines.indexed) ...[
                      if (i > 0) const SizedBox(height: 12),
                      line,
                    ],
                  ],
                ),
              ),
            ),
      actions: nothing
          ? [
              FilledButton(
                onPressed: () => Navigator.of(context).pop(),
                child: const Text('OK'),
              ),
            ]
          : [
              TextButton(
                onPressed: working ? null : () => Navigator.of(context).pop(),
                child: const Text('Cancel'),
              ),
              WorkingButton(
                label: action.button,
                working: working,
                onPressed: go,
              ),
            ],
    );
  }
}

/// Says that some of the items weren't done, and why: never only a note
/// that goes away.
Future<void> showBulkFailure(BuildContext context, BulkOutcome outcome) =>
    showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(outcome.action.didnt(outcome.failed.length)),
        content: SizedBox(
          width: 380,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (outcome.error case final error?) ...[
                Text(error),
                const SizedBox(height: 12),
              ],
              Text(
                outcome.said.isEmpty
                    ? 'Nothing was changed. Still selected, to try again.'
                    : '${outcome.said} What didn\'t go is still selected, '
                          'to try again.',
              ),
            ],
          ),
        ),
        actions: [
          FilledButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('OK'),
          ),
        ],
      ),
    );
