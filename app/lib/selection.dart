// Several items selected in a list, and one thing done to all of them
// (docs/core.md, "Selecting several items"): favorites, Archive, a folder or
// shared folder, the trash; and in the trash, restoring and deleting for
// good. The lists keep which items are selected (vault.dart); here is what's
// done with them: the bar with the actions, where to move them, the
// question asked first, and the words for what happened.
//
// What a person may do with each item is the core's to say (`Core.bulkPlan`
// before anything is sent, `Core.bulk` after): nothing here decides it. The
// items left as they are get said with why, before and after. In "All
// accounts" each item is acted on in its own account, with that account's
// own call.
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

/// An item in a selection, by its account on this device and its id: "All
/// accounts" lists several accounts' items together, and two accounts in
/// one organization list its items under the same id.
String keyOf(String account, String id) => '$account $id';

/// A listed item's key ([keyOf]): its own account in "All accounts", else
/// the account the screens show.
String selectionKey(Map<String, dynamic> item) =>
    keyOf(item['account'] as String? ?? Core.account, item['id'] as String);

/// A key's account and item id.
(String, String) splitKey(String key) {
  final at = key.indexOf(' ');
  return (key.substring(0, at), key.substring(at + 1));
}

/// The items of a selection by account, each account's in the order given.
Map<String, List<String>> byAccount(Iterable<String> keys) {
  final by = <String, List<String>>{};
  for (final key in keys) {
    final (account, id) = splitKey(key);
    by.putIfAbsent(account, () => []).add(id);
  }
  return by;
}

enum BulkKind {
  favorite,
  unfavorite,
  archive,
  unarchive,
  trash,
  restore,
  deleteForever,
  folder,
  sharedFolder,
}

/// One thing to do to every selected item.
class BulkAction {
  const BulkAction._(this.kind)
    : to = null,
      toName = null,
      from = null,
      owner = null;

  const BulkAction.favorite(bool on)
    : this._(on ? BulkKind.favorite : BulkKind.unfavorite);

  /// Into Archive, or out of it ([on] false): the person's own mark.
  const BulkAction.archive(bool on)
    : this._(on ? BulkKind.archive : BulkKind.unarchive);

  const BulkAction.trash() : this._(BulkKind.trash);

  /// Out of the trash, back where they were.
  const BulkAction.restore() : this._(BulkKind.restore);

  /// Out of the trash for good.
  const BulkAction.deleteForever() : this._(BulkKind.deleteForever);

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
    BulkKind.archive => {'do': 'archive', 'archived': true},
    BulkKind.unarchive => {'do': 'archive', 'archived': false},
    BulkKind.trash => {'do': 'trash'},
    BulkKind.restore => {'do': 'restore'},
    BulkKind.deleteForever => {'do': 'deleteForever'},
    BulkKind.folder => {'do': 'folder', 'folderId': to},
    BulkKind.sharedFolder => {'do': 'sharedFolder', 'to': to, 'from': from},
  };

  /// It asks first, every time, with how many: the trash, and deleting for
  /// good, which can't be undone.
  bool get alwaysAsks =>
      kind == BulkKind.trash || kind == BulkKind.deleteForever;

  /// Its last step is the red one.
  bool get danger => alwaysAsks;

  /// What it's done to leaves the list it's in: Archive, the trash, and
  /// the trash's own actions.
  bool get leavesTheList => const {
    BulkKind.archive,
    BulkKind.unarchive,
    BulkKind.trash,
    BulkKind.restore,
    BulkKind.deleteForever,
  }.contains(kind);

  /// On its button.
  String get button => switch (kind) {
    BulkKind.favorite => 'Add to favorites',
    BulkKind.unfavorite => 'Remove from favorites',
    BulkKind.archive => 'Archive',
    BulkKind.unarchive => 'Move out of Archive',
    BulkKind.trash => 'Move to trash',
    BulkKind.restore => 'Restore',
    BulkKind.deleteForever => 'Delete forever',
    BulkKind.folder || BulkKind.sharedFolder => 'Move',
  };

  String get _verb => switch (kind) {
    BulkKind.favorite => 'Add',
    BulkKind.unfavorite => 'Remove',
    BulkKind.restore => 'Restore',
    BulkKind.deleteForever => 'Delete',
    _ => 'Move',
  };

  String get _past => switch (kind) {
    BulkKind.favorite => 'added',
    BulkKind.unfavorite => 'removed',
    BulkKind.restore => 'restored',
    BulkKind.deleteForever => 'deleted',
    _ => 'moved',
  };

  /// Where they go, after the verb and how many: " to the trash", or
  /// nothing for restoring.
  String get _where => switch (kind) {
    BulkKind.favorite => ' to Favorites',
    BulkKind.unfavorite => ' from Favorites',
    BulkKind.archive => ' to Archive',
    BulkKind.unarchive => ' out of Archive',
    BulkKind.trash => ' to the trash',
    BulkKind.restore => '',
    BulkKind.deleteForever => ' forever',
    BulkKind.folder when to == null => ' to Personal, in no folder',
    BulkKind.folder || BulkKind.sharedFolder => ' to $toName',
  };

  /// "Move 3 items to the trash?", "Restore 3 items?"
  String ask(int n) => '$_verb ${items(n)}$_where?';

  /// "Moved 3 items to the trash.", "Restored 3 items."
  String did(int n) =>
      '${_past[0].toUpperCase()}${_past.substring(1)} ${items(n)}$_where.';

  /// "None of these 3 items can be moved to the trash."
  String cant(int n) => n == 1
      ? 'This item can\'t be $_past$_where'
      : 'None of these $n items can be $_past$_where';

  /// "2 items weren't moved to the trash."
  String didnt(int n) => n == 1
      ? '1 item wasn\'t $_past$_where'
      : '$n items weren\'t $_past$_where';

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
      // Items that ask for it, in more than one account: one master
      // password doesn't include them all (`planAcross`).
      'manyMasterPasswords' =>
        '${one ? 'it asks' : 'they ask'} for the master password of '
            '${one ? 'its account' : 'their accounts'}; select one '
            'account\'s items at a time to include $it',
      'noDelete' => 'your organization doesn\'t let you delete $it',
      'noRestore' => 'your organization doesn\'t let you restore $it',
      'notInTrash' =>
        one
            ? 'it isn\'t in the trash anymore'
            : 'they aren\'t in the trash anymore',
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

/// Each item an answer leaves as it is, with its key ([keyOf]) for its id
/// and its account; those that ask for the master password said apart when
/// they're in more than one account, since one password can't include
/// them all.
List<Map<String, dynamic>> _leftIn(
  Map<String, Answer> answers,
) {
  final left = [
    for (final MapEntry(key: account, value: answer) in answers.entries)
      for (final s in ((answer.body['skipped'] as List?) ?? const []))
        {
          'id': keyOf(account, (s as Map)['id'] as String),
          'why': s['why'],
          'account': account,
        },
  ];
  final asking = {
    for (final s in left)
      if (s['why'] == 'masterPassword') s['account'],
  };
  if (asking.length < 2) return left;
  return [
    for (final s in left)
      s['why'] == 'masterPassword' ? {...s, 'why': 'manyMasterPasswords'} : s,
  ];
}

/// What [action] would do to the selected items ([ids], by account), each
/// in its own account, from the vaults kept here with nothing sent
/// (`Core.bulkPlan`): one answer as one account's is, with keys ([keyOf])
/// for ids, and each item it leaves with its `account`. An account whose
/// plan isn't ok answers for all.
Answer planAcross(
  Core core,
  Map<String, List<String>> ids,
  Map<String, dynamic> action,
) {
  final plans = <String, Answer>{};
  for (final MapEntry(key: account, value: list) in ids.entries) {
    final plan = core.bulkPlan(list, action, account);
    if (!plan.ok) return plan;
    plans[account] = plan;
  }
  return Answer({
    'ok': true,
    'go': [
      for (final MapEntry(key: account, value: plan) in plans.entries)
        for (final id in _ids(plan, 'go')) keyOf(account, id),
    ],
    'skipped': _leftIn(plans),
  });
}

/// The account whose master password includes the items a plan
/// ([planAcross]) leaves for wanting it: null when there are none, or
/// they're in more than one account.
String? askingIn(Answer plan) {
  final accounts = {
    for (final s in ((plan.body['skipped'] as List?) ?? const []))
      if ((s as Map)['why'] == 'masterPassword') s['account'] as String?,
  };
  return accounts.length == 1 ? accounts.single : null;
}

/// Does [action] to the selected items ([ids], by account), each in its
/// own account (`Core.bulk`), each account's then syncing. [password] is
/// for the items of [passwordFor] that ask for it: that account goes first,
/// so a wrong one stops everything before anything is done, as with one
/// account. One answer as one account's is, with keys ([keyOf]) for ids.
/// An account the call fails for (signed out there, say) has its items
/// among `failed`, with its words; when it fails for every account, the
/// answer is the first one's.
Future<Answer> bulkAcross(
  Map<String, List<String>> ids,
  Map<String, dynamic> action, {
  String password = '',
  String? passwordFor,
}) async {
  final accounts = [
    if (passwordFor != null && ids.containsKey(passwordFor)) passwordFor,
    for (final account in ids.keys)
      if (account != passwordFor) account,
  ];
  final answered = <String, Answer>{};
  final done = <String>[];
  final failed = <String>[];
  String? error;
  var offline = false;
  var signedOut = false;
  var synced = true;
  Answer? refused;
  for (final account in accounts) {
    final given = account == passwordFor ? password : '';
    final answer = await Core.bulk(
      ids[account]!,
      action,
      password: given,
      forAccount: account,
    );
    offline |= answer.offline;
    signedOut |= answer.signedOut;
    if (!answer.ok) {
      // A wrong master password does nothing at all.
      if (given.isNotEmpty && !answer.offline && !answer.signedOut) {
        return answer;
      }
      refused ??= answer;
      failed.addAll([for (final id in ids[account]!) keyOf(account, id)]);
      error ??= answer.error;
      continue;
    }
    answered[account] = answer;
    done.addAll([for (final id in _ids(answer, 'done')) keyOf(account, id)]);
    failed.addAll([
      for (final id in _ids(answer, 'failed')) keyOf(account, id),
    ]);
    error ??= answer.error;
    synced &= answer.body['synced'] != false;
  }
  if (answered.isEmpty && refused != null) return refused;
  return Answer({
    'ok': true,
    'done': done,
    'skipped': _leftIn(answered),
    'failed': failed,
    'error': error,
    'offline': offline,
    'signedOut': signedOut,
    'synced': synced,
  });
}

/// What a bulk action did ([bulkAcross]' answer), in words: what went, what
/// was left and why, and what didn't go. [done] and [failed] are keys
/// ([keyOf]).
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

/// One of the selection bar's actions: its words (a computer's tooltip, a
/// phone's label), its icon, and what it does.
class BarAction {
  const BarAction(this.label, this.icon, this.onPressed);

  final String label;
  final Widget icon;
  final VoidCallback onPressed;
}

/// A list's header on a computer while items are selected, or while Select
/// is on: how many, and what can be done with them ([actions]: only those
/// the person may do with any of them, and none while nothing is selected).
class SelectionBar extends StatelessWidget {
  const SelectionBar({
    super.key,
    required this.count,
    required this.onClear,
    required this.onSelectAll,
    this.actions = const [],
  });

  final int count;
  final VoidCallback onClear;
  final VoidCallback onSelectAll;
  final List<BarAction> actions;

  @override
  Widget build(BuildContext context) {
    final shown = count == 0 ? const <BarAction>[] : actions;
    return Padding(
      padding: const EdgeInsets.fromLTRB(10, 4, 10, 0),
      child: LayoutBuilder(
        builder: (context, box) {
          // Every button fits the narrowest window: smaller there, with room
          // left for the number.
          final size = ((box.maxWidth - 30) / (shown.length + 2)).clamp(
            28.0,
            40.0,
          );
          Widget button(String tooltip, Widget icon, VoidCallback onPressed) =>
              IconButton(
                tooltip: tooltip,
                onPressed: onPressed,
                style: IconButton.styleFrom(
                  minimumSize: Size.square(size),
                  fixedSize: Size.square(size),
                  padding: EdgeInsets.zero,
                  tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                ),
                icon: icon,
              );
          final said = count == 0 ? 'Choose items' : '$count selected';
          return Row(
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
                  box.maxWidth < 300 && count > 0 ? '$count' : said,
                  semanticsLabel: said,
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
              for (final a in shown) button(a.label, a.icon, a.onPressed),
            ],
          );
        },
      ),
    );
  }
}

/// A phone's bar at the bottom while items are selected, where the tabs
/// are otherwise: what can be done with them.
class PhoneSelectionBar extends StatelessWidget {
  const PhoneSelectionBar({super.key, required this.actions});

  final List<BarAction> actions;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget action(BarAction a) => Expanded(
      child: InkWell(
        onTap: a.onPressed,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 10),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              a.icon,
              const SizedBox(height: 6),
              Text(
                a.label,
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
            child: Row(children: [for (final a in actions) action(a)]),
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

/// Asks before doing [action] to the selected items ([ids], by account),
/// saying how many it's for and how many are left as they are and why
/// ([plan], from [planAcross]), then does it ([bulkAcross]). Items that ask
/// for the master password are included once it's given here, one time
/// for all of them, when they're all in one account; [passwordOf] names
/// that account for the field when it isn't the one the screens show. The
/// answer once it's done, or null if the person didn't go on.
Future<Answer?> confirmBulk(
  BuildContext context, {
  required BulkAction action,
  required Map<String, List<String>> ids,
  required Answer plan,
  String? passwordOf,
}) => showDialog<Answer>(
  context: context,
  builder: (_) => _BulkDialog(
    action: action,
    ids: ids,
    plan: plan,
    passwordOf: passwordOf,
  ),
);

class _BulkDialog extends StatefulWidget {
  const _BulkDialog({
    required this.action,
    required this.ids,
    required this.plan,
    this.passwordOf,
  });

  final BulkAction action;
  final Map<String, List<String>> ids;
  final Answer plan;
  final String? passwordOf;

  @override
  State<_BulkDialog> createState() => _BulkDialogState();
}

class _BulkDialogState extends State<_BulkDialog> {
  final password = TextEditingController();
  String? error;
  bool working = false;

  /// The items it's done for whatever is typed here.
  late final sure = ((widget.plan.body['go'] as List?) ?? const []).length;

  /// The account whose master password includes the items that ask for it.
  late final passwordFor = askingIn(widget.plan);

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
      () => bulkAcross(
        widget.ids,
        widget.action.json,
        password: given,
        passwordFor: passwordFor,
      ),
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
    final all = [for (final list in widget.ids.values) ...list].length;
    final yours = switch (widget.passwordOf) {
      null => 'your master password',
      final name => 'the master password for $name',
    };
    final field = TextField(
      controller: password,
      obscureText: true,
      autofocus: sure == 0,
      decoration: InputDecoration(
        labelText: widget.passwordOf == null
            ? 'Master password'
            : 'Master password for ${widget.passwordOf}',
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
              ? '1 more item asks for $yours. Enter it to include that '
                    'item, or leave it empty to leave the item as it is.'
              : '$asking more items ask for $yours. Enter it to include '
                    'them, or leave it empty to leave them as they are.',
        ),
        field,
      ] else if (asking > 0) ...[
        Text(
          asking == 1
              ? 'This item asks for $yours.'
              : 'These items ask for $yours.',
        ),
        field,
      ],
      if (action.kind == BulkKind.trash && !nothing)
        Text(
          count == 1 && asking == 0
              ? 'You can restore it from the trash.'
              : 'You can restore them from the trash.',
        ),
      if (action.kind == BulkKind.deleteForever && !nothing)
        Text(
          count == 1 && asking == 0
              ? 'It\'s deleted for good, with its attachments and history. '
                    'This can\'t be undone.'
              : 'They\'re deleted for good, with their attachments and '
                    'history. This can\'t be undone.',
        ),
    ];
    return AlertDialog(
      title: Text(nothing ? action.cant(all) : action.ask(count)),
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
                // The trash, and deleting for good: the red last step.
                style: action.danger ? dangerButton(context) : null,
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
