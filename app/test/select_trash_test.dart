// Selecting several items with Select, Archive for several, the trash as a
// place of its own (restoring and deleting for good, one item or several),
// and "All accounts" acting on each item in its own account (docs/core.md in
// the meta repo, "Selecting several items"). Against the real core, and for
// what the server was asked, a stand-in for it on this computer.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/selection.dart';
import 'package:passbundle/vault.dart';

import 'support.dart';

// The sample vault's items.
const officeWifi = '4e5f6071-8293-4a41-bcd2-e3f405162738';
const backup = '5f607182-93a4-4b52-cde3-f40516273849';
const gate = '60718293-a4b5-4c63-def4-05162738495a';
const card = '718293a4-b5c6-4d74-8f01-162738495a6b';

/// When an item went to the trash, as the server says it.
const trashedAt = '2026-11-09T10:00:00.000Z';

/// What the server says of an item the organization doesn't let the person
/// delete, nor restore.
const noDelete = {
  'permissions': {'delete': false, 'restore': false},
};

/// A stand-in for PassBundle's server: the sample vault's account
/// (test/sample), kept and changed as the server's calls for several items
/// change it: Archive, the trash, restoring, and deleting for good. [asked]
/// has each call, and [sent] what came with it.
class StandIn {
  StandIn._(this._server);

  final HttpServer _server;
  final asked = <String>[];
  final sent = <String, Object?>{};
  final Map<String, dynamic> sync = jsonDecode(
    File('test/sample/sync.json').readAsStringSync(),
  ) as Map<String, dynamic>;

  String get url => 'http://127.0.0.1:${_server.port}';

  List<Map<String, dynamic>> get ciphers =>
      (sync['ciphers'] as List).cast<Map<String, dynamic>>();

  Map<String, dynamic> item(String id) =>
      ciphers.firstWhere((c) => c['id'] == id);

  /// What it was asked to change, in order.
  List<String> get changes => [
    for (final a in asked)
      if (a.startsWith('PUT ') || a.startsWith('DELETE ')) a,
  ];

  /// The ids sent with a call, in no order.
  Set<String> idsSent(String call) =>
      (((sent[call] as Map?)?['ids'] as List?) ?? const [])
          .cast<String>()
          .toSet();

  static Future<StandIn> start(WidgetTester tester) async {
    final standIn = (await tester.runAsync(() async {
      final made = StandIn._(
        await HttpServer.bind(InternetAddress.loopbackIPv4, 0),
      );
      made._server.listen(made._answer);
      return made;
    }))!;
    addTearDown(
      () => tester.runAsync(() => standIn._server.close(force: true)),
    );
    return standIn;
  }

  Future<void> _answer(HttpRequest request) async {
    final path = request.uri.path;
    final call = '${request.method} $path';
    asked.add(call);
    final text = await utf8.decoder.bind(request).join();
    // Signing in comes as a form; only what changes items is kept.
    final changing = request.method == 'PUT' || request.method == 'DELETE';
    final body = changing && text.isNotEmpty ? jsonDecode(text) : null;
    if (changing) sent[call] = body;
    void json(int status, Object? value) {
      request.response.statusCode = status;
      request.response.headers.contentType = ContentType.json;
      if (value != null) request.response.write(jsonEncode(value));
    }

    final sentIds = body is Map ? body['ids'] as List? : null;
    final ids = [for (final id in sentIds ?? const []) id as String];
    void each(String name, Object? value) {
      for (final id in ids) {
        item(id)[name] = value;
      }
    }

    const list = {'data': <Object>[], 'object': 'list'};
    switch (path) {
      case '/api/config':
        json(200, {'version': '2026.6.0'});
      case '/identity/accounts/prelogin':
        json(200, {
          'kdf': 0,
          'kdfIterations': 600000,
          'kdfMemory': null,
          'kdfParallelism': null,
        });
      case '/identity/connect/token':
        json(200, {
          'access_token': 'a.b.c',
          'expires_in': 3600,
          'refresh_token': 'refresh',
          'token_type': 'Bearer',
          'Key': (sync['profile'] as Map)['key'],
        });
      case '/api/sync':
        json(200, sync);
      case '/api/ciphers/delete':
        each('deletedDate', trashedAt);
        json(200, null);
      case '/api/ciphers/restore':
        each('deletedDate', null);
        json(200, list);
      case '/api/ciphers/archive':
        each('archivedDate', trashedAt);
        json(200, list);
      case '/api/ciphers/unarchive':
        each('archivedDate', null);
        json(200, list);
      case '/api/ciphers' when request.method == 'DELETE':
        ciphers.removeWhere((c) => ids.contains(c['id']));
        json(200, null);
      default:
        json(404, null);
    }
    await request.response.close();
  }
}

void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

void main() {
  setUpAll(loadFonts);

  const computer = Size(1280, 800);
  const phone = Size(390, 844);

  /// The sample vault, unlocked, with no server behind it.
  Future<void> unlock(
    WidgetTester tester,
    Size size, {
    Map<String, Map<String, dynamic>> changed = const {},
  }) async {
    setSize(tester, size);
    openSample(changed: changed);
    await tester.pumpWidget(const PassBundleApp());
    await tester.enterText(find.byType(TextField), samplePassword);
    await tester.tap(find.text('Unlock'));
    await pumpUntil(
      tester,
      find.textContaining('Sign in again to get changes'),
    );
  }

  /// Signed in to a stand-in for the server, on a computer; [first] changes
  /// what it keeps before then.
  Future<StandIn> signIn(
    WidgetTester tester, {
    void Function(StandIn)? first,
  }) async {
    final standIn = await StandIn.start(tester);
    first?.call(standIn);
    openEmpty();
    setSize(tester, computer);
    await tester.pumpWidget(const PassBundleApp());
    await tester.tap(find.text('Self-hosted?'));
    await tester.pump();
    await tester.enterText(
      find.widgetWithText(TextField, 'Server'),
      standIn.url,
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Email'),
      'ana.lopez@example.test',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Master password'),
      samplePassword,
    );
    await tester.tap(find.text('Sign in'));
    await pumpUntil(tester, find.text('Example Mail'));
    // The sync as it opens, done.
    await pumpUntil(tester, find.byTooltip('Sync'));
    return standIn;
  }

  // A computer's list.
  Finder row(String name) => find.widgetWithText(ItemRow, name);
  List<String> names(WidgetTester tester) => [
    for (final r in tester.widgetList<ItemRow>(find.byType(ItemRow)))
      r.item['name'] as String,
  ];
  List<String> checked(WidgetTester tester) => [
    for (final r in tester.widgetList<ItemRow>(find.byType(ItemRow)))
      if (r.checked) r.item['name'] as String,
  ];
  Finder inBar(String tooltip) => find.descendant(
    of: find.byType(SelectionBar),
    matching: find.byTooltip(tooltip),
  );
  Finder inDialog(Finder what) =>
      find.descendant(of: find.byType(AlertDialog), matching: what);
  Finder selectButton() => find.widgetWithText(TextButton, 'Select');

  /// How many a menu entry says it holds.
  Finder countOf(Finder entry, String count) => find.descendant(
    of: find.ancestor(of: entry, matching: find.byType(InkWell)).first,
    matching: find.text(count),
  );

  /// A click with a key held.
  Future<void> clickWith(
    WidgetTester tester,
    LogicalKeyboardKey key,
    Finder what,
  ) async {
    await tester.sendKeyDownEvent(key);
    await tester.tap(what);
    await tester.sendKeyUpEvent(key);
    await tester.pump();
  }

  const ctrl = LogicalKeyboardKey.controlLeft;

  // A phone's list.
  Finder phoneRow(String name) => find.widgetWithText(PhoneItemRow, name);

  testWidgets(
    'Select, beside the count, shows every row\'s checkbox, a click then selects without opening, and Escape lets go of it all',
    (tester) async {
      await unlock(tester, computer);
      final all = names(tester);
      expect(all, hasLength(7));
      expect(find.byType(Checkbox), findsNothing);

      await tester.tap(selectButton());
      await tester.pump();
      expect(find.byType(SelectionBar), findsOneWidget);
      expect(find.text('Choose items'), findsOneWidget);
      expect(find.byType(Checkbox), findsNWidgets(all.length));
      // Nothing to do yet, with nothing selected.
      for (final action in ['Add to favorites', 'Archive', 'Move to trash']) {
        expect(inBar(action), findsNothing);
      }

      await tester.tap(row('Gate code'));
      await tester.pump();
      expect(checked(tester), ['Gate code']);
      expect(find.text('1 selected'), findsOneWidget);
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsNothing);
      for (final action in [
        'Add to favorites',
        'Move',
        'Archive',
        'Move to trash',
      ]) {
        expect(inBar(action), findsOneWidget);
      }
      // Taken out again, Select stays on.
      await tester.tap(row('Gate code'));
      await tester.pump();
      expect(checked(tester), isEmpty);
      expect(find.text('Choose items'), findsOneWidget);

      // The list has the keys: Ctrl+A takes everything, Escape lets go and
      // ends Select.
      await tester.sendKeyDownEvent(ctrl);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
      await tester.sendKeyUpEvent(ctrl);
      await tester.pump();
      expect(checked(tester), all);
      expect(find.text('7 selected'), findsOneWidget);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      expect(checked(tester), isEmpty);
      expect(find.byType(SelectionBar), findsNothing);
      expect(find.byType(Checkbox), findsNothing);

      // A click opens an item again.
      await tester.tap(row('Gate code'));
      await tester.pump();
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsOneWidget);
    },
  );

  testWidgets(
    'Archive takes several items in one call, says how many with Undo, and Move out of Archive brings them back from Archive\'s own list',
    (tester) async {
      final standIn = await signIn(tester);
      await clickWith(tester, ctrl, row('Gate code'));
      await clickWith(tester, ctrl, row('Everyday card'));
      await clickWith(tester, ctrl, row('Office Wi-Fi'));
      // No question: every item the person sees takes their own mark.
      await tester.tap(inBar('Archive'));
      await pumpUntil(tester, find.text('Moved 3 items to Archive.'));
      expect(standIn.changes, ['PUT /api/ciphers/archive']);
      expect(standIn.idsSent('PUT /api/ciphers/archive'), {
        gate,
        card,
        officeWifi,
      });
      expect(names(tester), hasLength(4));
      expect(names(tester), isNot(contains('Gate code')));
      expect(find.byType(SelectionBar), findsNothing);

      // Undo: the same three, out again, in one call.
      await tester.pumpAndSettle();
      await tester.tap(find.text('Undo'));
      await pumpUntil(tester, find.text('Moved 3 items out of Archive.'));
      expect(standIn.changes, [
        'PUT /api/ciphers/archive',
        'PUT /api/ciphers/unarchive',
      ]);
      expect(standIn.idsSent('PUT /api/ciphers/unarchive'), {
        gate,
        card,
        officeWifi,
      });
      expect(names(tester), hasLength(7));

      // Into Archive again, then out from Archive's own list, where the
      // bar offers Move out of Archive.
      standIn.asked.clear();
      await clickWith(tester, ctrl, row('Gate code'));
      await clickWith(tester, ctrl, row('Everyday card'));
      await tester.tap(inBar('Archive'));
      await pumpUntil(tester, find.text('Moved 2 items to Archive.'));
      final archive = await menuEntry(tester, 'Archive');
      expect(countOf(archive, '2'), findsOneWidget);
      await tester.tap(archive);
      await tester.pump();
      expect(names(tester), ['Everyday card', 'Gate code']);
      await tester.tap(selectButton());
      await tester.pump();
      await tester.tap(inBar('Select all'));
      await tester.pump();
      expect(inBar('Archive'), findsNothing);
      await tester.tap(inBar('Move out of Archive'));
      await pumpUntil(tester, find.text('Moved 2 items out of Archive.'));
      expect(standIn.changes, [
        'PUT /api/ciphers/archive',
        'PUT /api/ciphers/unarchive',
      ]);
      expect(
        find.text(
          'Nothing in Archive. An archived item stays in your vault, out of the lists, search, and filling.',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'the trash is a place of its own, with its count and its search, in no other list or search; Restore and Delete forever take several in one call each',
    (tester) async {
      final standIn = await signIn(
        tester,
        first: (s) {
          for (final id in [gate, card, backup]) {
            s.item(id)['deletedDate'] = trashedAt;
          }
        },
      );
      // Not in All items, nor found by its search.
      expect(names(tester), hasLength(4));
      final search = find.byType(TextField);
      await tester.enterText(search, 'gate');
      await tester.pump();
      expect(names(tester), isEmpty);
      await tester.enterText(search, '');
      await tester.pump();

      // The menu's Trash, with how many it holds.
      final trash = await menuEntry(tester, 'Trash');
      expect(countOf(trash, '3'), findsOneWidget);
      await tester.tap(trash);
      await tester.pump();
      expect(names(tester), ['Backup service', 'Everyday card', 'Gate code']);
      await tester.enterText(search, 'gate');
      await tester.pump();
      expect(names(tester), ['Gate code']);
      await tester.enterText(search, '');
      await tester.pump();

      // The trash's actions only.
      await clickWith(tester, ctrl, row('Gate code'));
      await clickWith(tester, ctrl, row('Everyday card'));
      expect(inBar('Restore'), findsOneWidget);
      expect(inBar('Delete forever'), findsOneWidget);
      for (final none in [
        'Add to favorites',
        'Move',
        'Archive',
        'Move to trash',
      ]) {
        expect(inBar(none), findsNothing);
      }
      // Restoring asks nothing when every one may be restored.
      await tester.tap(inBar('Restore'));
      await pumpUntil(tester, find.text('Restored 2 items.'));
      expect(standIn.changes, ['PUT /api/ciphers/restore']);
      expect(standIn.idsSent('PUT /api/ciphers/restore'), {gate, card});
      expect(names(tester), ['Backup service']);
      expect(countOf(trash, '1'), findsOneWidget);

      // Deleting for good asks first, and says it can't be undone.
      await clickWith(tester, ctrl, row('Backup service'));
      await tester.tap(inBar('Delete forever'));
      await tester.pumpAndSettle();
      expect(find.text('Delete 1 item forever?'), findsOneWidget);
      expect(
        find.text(
          'It\'s deleted for good, with its attachments and history. This can\'t be undone.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(standIn.changes, ['PUT /api/ciphers/restore']);
      expect(checked(tester), ['Backup service']);
      await tester.tap(inBar('Delete forever'));
      await tester.pumpAndSettle();
      await tester.tap(inDialog(find.text('Delete forever')));
      await pumpUntil(tester, find.text('Deleted 1 item forever.'));
      expect(standIn.changes, [
        'PUT /api/ciphers/restore',
        'DELETE /api/ciphers',
      ]);
      expect(standIn.idsSent('DELETE /api/ciphers'), {backup});
      expect(
        find.text(
          'Nothing in the trash. An item moved to the trash waits here until you restore it or delete it forever.',
        ),
        findsOneWidget,
      );

      // The two restored are back in All items; the one deleted isn't.
      await tester.tap(
        find.descendant(
          of: find.byType(Sidebar),
          matching: find.text('All items'),
        ),
      );
      await tester.pump();
      expect(names(tester), hasLength(6));
      expect(names(tester), containsAll(['Gate code', 'Everyday card']));
      expect(names(tester), isNot(contains('Backup service')));
    },
  );

  testWidgets(
    'moving several to the trash says where they went, and Open trash shows them there',
    (tester) async {
      final standIn = await signIn(tester);
      await clickWith(tester, ctrl, row('Gate code'));
      await clickWith(tester, ctrl, row('Everyday card'));
      await tester.tap(inBar('Move to trash'));
      await tester.pumpAndSettle();
      expect(find.text('Move 2 items to the trash?'), findsOneWidget);
      expect(find.text('You can restore them from the trash.'), findsOneWidget);
      await tester.tap(inDialog(find.text('Move to trash')));
      await pumpUntil(tester, find.text('Moved 2 items to the trash.'));
      expect(standIn.changes, ['PUT /api/ciphers/delete']);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open trash'));
      await tester.pump();
      expect(names(tester), ['Everyday card', 'Gate code']);
    },
  );

  testWidgets(
    'in the trash, an item the organization doesn\'t let the person delete has no Restore or Delete forever, and the question says how many are left and why',
    (tester) async {
      await unlock(
        tester,
        computer,
        changed: {
          officeWifi: {...noDelete, 'deletedDate': trashedAt},
          gate: {'deletedDate': trashedAt},
        },
      );
      await tester.tap(await menuEntry(tester, 'Trash'));
      await tester.pump();
      expect(names(tester), ['Gate code', 'Office Wi-Fi']);
      await clickWith(tester, ctrl, row('Office Wi-Fi'));
      expect(inBar('Restore'), findsNothing);
      expect(inBar('Delete forever'), findsNothing);

      await tester.tap(inBar('Select all'));
      await tester.pump();
      await tester.tap(inBar('Restore'));
      await tester.pumpAndSettle();
      expect(find.text('Restore 1 item?'), findsOneWidget);
      expect(
        find.text(
          '1 item will be left as it is: your organization doesn\'t let you restore it.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      await tester.tap(inBar('Delete forever'));
      await tester.pumpAndSettle();
      expect(find.text('Delete 1 item forever?'), findsOneWidget);
      expect(
        find.text(
          '1 item will be left as it is: your organization doesn\'t let you delete it.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(checked(tester), ['Gate code', 'Office Wi-Fi']);

      // Opened, an item in the trash has the two, and only where the
      // person may delete it.
      await tester.tap(inBar('Clear the selection'));
      await tester.pump();
      await tester.tap(row('Gate code'));
      await tester.pump();
      expect(find.byTooltip('Restore'), findsOneWidget);
      expect(find.byTooltip('Delete forever'), findsOneWidget);
      for (final none in ['Move to trash', 'Archive', 'Add to favorites']) {
        expect(find.byTooltip(none), findsNothing);
      }
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsNothing);
      await tester.tap(row('Office Wi-Fi'));
      await tester.pump();
      expect(find.byTooltip('Restore'), findsNothing);
      expect(find.byTooltip('Delete forever'), findsNothing);
    },
  );

  testWidgets(
    'an item opened in the trash is restored, or deleted for good after a question, on its own',
    (tester) async {
      final standIn = await signIn(
        tester,
        first: (s) {
          s.item(gate)['deletedDate'] = trashedAt;
          s.item(card)['deletedDate'] = trashedAt;
        },
      );
      await tester.tap(await menuEntry(tester, 'Trash'));
      await tester.pump();
      await tester.tap(row('Gate code'));
      await tester.pump();
      await tester.tap(find.byTooltip('Restore'));
      await pumpUntil(tester, find.text('Restored Gate code.'));
      expect(standIn.changes, ['PUT /api/ciphers/restore']);
      expect(standIn.idsSent('PUT /api/ciphers/restore'), {gate});
      expect(names(tester), ['Everyday card']);

      await tester.tap(row('Everyday card'));
      await tester.pump();
      await tester.tap(find.byTooltip('Delete forever'));
      await tester.pumpAndSettle();
      expect(find.text('Delete Everyday card forever?'), findsOneWidget);
      expect(
        find.text(
          'It\'s deleted for good, with its attachments and history. This can\'t be undone.',
        ),
        findsOneWidget,
      );
      await tester.tap(inDialog(find.text('Delete forever')));
      await pumpUntil(tester, find.text('Deleted Everyday card forever.'));
      expect(standIn.changes, [
        'PUT /api/ciphers/restore',
        'DELETE /api/ciphers',
      ]);
      expect(standIn.idsSent('DELETE /api/ciphers'), {card});
      expect(names(tester), isEmpty);
    },
  );

  testWidgets(
    'on a phone, the trash is a row with its count beside Archive, a long press there offers Restore and Delete forever, and elsewhere Archive',
    (tester) async {
      await unlock(tester, phone, changed: {gate: {'deletedDate': trashedAt}});
      final trashRow = find.byKey(const ValueKey('trash'));
      await tester.scrollUntilVisible(trashRow, 200, scrollable: vaultScroll);
      await tester.pumpAndSettle();
      expect(
        find.descendant(of: trashRow, matching: find.text('1')),
        findsOneWidget,
      );
      await tester.tap(trashRow);
      await tester.pumpAndSettle();
      expect(phoneRow('Gate code'), findsOneWidget);
      expect(phoneRow('Example Mail'), findsNothing);

      await tester.longPress(phoneRow('Gate code'));
      await tester.pumpAndSettle();
      expect(find.text('1 selected'), findsOneWidget);
      final bar = find.byType(PhoneSelectionBar);
      for (final action in ['Restore', 'Delete forever']) {
        expect(
          find.descendant(of: bar, matching: find.text(action)),
          findsOneWidget,
        );
      }
      for (final none in ['Move to trash', 'Archive', 'Move']) {
        expect(
          find.descendant(of: bar, matching: find.text(none)),
          findsNothing,
        );
      }
      await tester.tap(
        find.descendant(of: bar, matching: find.text('Delete forever')),
      );
      await tester.pumpAndSettle();
      expect(find.text('Delete 1 item forever?'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      // Back clears the selection, then leaves the trash.
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(find.byType(PhoneSelectionBar), findsNothing);
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Logins'));
      await tester.pumpAndSettle();
      await tester.longPress(phoneRow('Example Mail'));
      await tester.pumpAndSettle();
      for (final action in ['Move', 'Archive', 'Move to trash']) {
        expect(
          find.descendant(of: bar, matching: find.text(action)),
          findsOneWidget,
        );
      }
    },
  );

  testWidgets(
    'in All accounts, both accounts\' items are selected together, each planned in its own account, and Move shows only for one account\'s',
    (tester) async {
      setSize(tester, computer);
      final [w, p] = openAccounts();
      expect(Core().setAccountLabel('Work', null, w).ok, isTrue);
      expect(Core().setAccountLabel('Personal', null, p).ok, isTrue);
      await tester.pumpWidget(const PassBundleApp());
      Future<void> unlockShown(String shows) async {
        await tester.enterText(find.byType(TextField), samplePassword);
        await tester.tap(find.text('Unlock'));
        await pumpUntil(tester, find.text(shows));
        await pumpUntil(
          tester,
          find.textContaining('Sign in again to get changes'),
        );
        await tester.pumpAndSettle();
      }

      await unlockShown('Example Mail');
      await tester.tap(find.byTooltip('Accounts'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey('account-$p')));
      await tester.pumpAndSettle();
      await unlockShown('Gate code');
      await tester.tap(find.text('All accounts'));
      await tester.pumpAndSettle();

      // The personal account's own, which it shows: Move has its folders.
      await clickWith(tester, ctrl, row('Gate code'));
      expect(checked(tester), ['Gate code']);
      expect(inBar('Move'), findsOneWidget);
      // And the work account's: no Move across two accounts.
      await clickWith(tester, ctrl, row('Example Mail'));
      expect(checked(tester).toSet(), {'Example Mail', 'Gate code'});
      expect(inBar('Move'), findsNothing);
      expect(inBar('Archive'), findsOneWidget);
      expect(inBar('Move to trash'), findsOneWidget);
      // Each is planned in its own account: neither is left as it is.
      await tester.tap(inBar('Move to trash'));
      await tester.pumpAndSettle();
      expect(find.text('Move 2 items to the trash?'), findsOneWidget);
      expect(find.textContaining('left as'), findsNothing);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      // The work account's alone: Move, with its places.
      await clickWith(tester, ctrl, row('Gate code'));
      expect(checked(tester), ['Example Mail']);
      expect(inBar('Move'), findsOneWidget);

      // Select all takes every account's, and Escape lets go.
      await tester.tap(inBar('Select all'));
      await tester.pump();
      expect(checked(tester), names(tester));
      expect(checked(tester), containsAll(['Example Mail', 'Gate code']));
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      expect(find.byType(SelectionBar), findsNothing);
    },
  );
}
