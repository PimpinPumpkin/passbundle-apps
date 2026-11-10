// Several items selected at once, and one thing done to all of them
// (docs/core.md, "Selecting several items"): how they're selected on a
// computer (a checkbox under the pointer, Shift and Ctrl or Command with a
// click, Ctrl+A, Escape) and on a phone (a long press, then taps; back
// clears first), what's asked before, and what's said after, with the items
// left as they are and why. Against the real core, and for what happened a
// stand-in for the server on this computer.
import 'dart:convert';
import 'dart:io';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/main.dart';
import 'package:passbundle/selection.dart';
import 'package:passbundle/vault.dart';

import 'support.dart';

// The sample vault's items.
const mail = '3d4e5f60-7182-4930-abc1-d2e3f4051627';
const officeWifi = '4e5f6071-8293-4a41-bcd2-e3f405162738';
const backup = '5f607182-93a4-4b52-cde3-f40516273849';

/// What the server says of an item the organization doesn't let the person
/// delete.
const noDelete = {
  'permissions': {'delete': false, 'restore': false},
};

/// A stand-in for PassBundle's server: the sample vault's account
/// (test/sample), kept and changed as the server's calls for several items
/// change it. An item in [refuses] gets the server's refusal. [asked] has
/// each call, and [sent] what came with it.
class StandIn {
  StandIn._(this._server);

  final HttpServer _server;
  final asked = <String>[];
  final sent = <String, Object?>{};
  final refuses = <String>{};
  final Map<String, dynamic> sync = jsonDecode(
    File('test/sample/sync.json').readAsStringSync(),
  ) as Map<String, dynamic>;

  String get url => 'http://127.0.0.1:${_server.port}';

  Map<String, dynamic> item(String id) => (sync['ciphers'] as List)
      .cast<Map<String, dynamic>>()
      .firstWhere((c) => c['id'] == id);

  /// What it was asked to change, in order.
  List<String> get changes => [
    for (final a in asked)
      if (a.startsWith('PUT ')) a,
  ];

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
    final body = request.method == 'PUT' && text.isNotEmpty
        ? jsonDecode(text)
        : null;
    if (request.method == 'PUT') sent[call] = body;
    void json(int status, Object? value) {
      request.response.statusCode = status;
      request.response.headers.contentType = ContentType.json;
      if (value != null) request.response.write(jsonEncode(value));
    }

    final ids = body is Map ? ((body['ids'] as List?) ?? const []) : const [];
    final one = RegExp(r'^/api/ciphers/([^/]+)/(\w+)$').firstMatch(path);
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
        for (final id in ids) {
          item(id as String)['deletedDate'] = '2026-11-09T10:00:00.000Z';
        }
        json(200, null);
      case '/api/ciphers/move':
        for (final id in ids) {
          item(id as String)['folderId'] = (body as Map)['folderId'];
        }
        json(200, null);
      case _ when one != null && refuses.contains(one.group(1)):
        json(400, {'message': 'Cipher is not write accessible'});
      case _ when one?.group(2) == 'partial':
        item(one!.group(1)!)
          ..['favorite'] = (body as Map)['favorite']
          ..['folderId'] = body['folderId'];
        json(200, {});
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

  /// The sample vault, unlocked, with no server behind it.
  Future<void> unlock(
    WidgetTester tester,
    Size size, {
    Set<String> asking = const {},
    Map<String, Map<String, dynamic>> changed = const {},
  }) async {
    setSize(tester, size);
    openSample(asking: asking, changed: changed);
    await tester.pumpWidget(const PassBundleApp());
    await tester.enterText(find.byType(TextField), samplePassword);
    await tester.tap(find.text('Unlock'));
    await pumpUntil(
      tester,
      find.textContaining('Sign in again to get changes'),
    );
  }

  const computer = Size(1280, 800);
  const phone = Size(390, 844);

  /// Signed in to a stand-in for the server, on a computer.
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

  const shift = LogicalKeyboardKey.shiftLeft;
  const ctrl = LogicalKeyboardKey.controlLeft;

  // A phone's list.
  Finder phoneRow(String name) => find.widgetWithText(PhoneItemRow, name);
  List<String> phoneChecked(WidgetTester tester) => [
    for (final r in tester.widgetList<PhoneItemRow>(find.byType(PhoneItemRow)))
      if (r.checked == true) r.item['name'] as String,
  ];

  testWidgets(
    'on a computer, a row\'s checkbox shows under the pointer, and a click on it selects the item without opening it',
    (tester) async {
      await unlock(tester, computer);
      expect(find.byType(Checkbox), findsNothing);

      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      addTearDown(mouse.removePointer);
      await mouse.moveTo(tester.getCenter(row('Gate code')));
      await tester.pump();
      expect(find.byType(Checkbox), findsOneWidget);
      expect(
        find.descendant(of: row('Gate code'), matching: find.byType(Checkbox)),
        findsOneWidget,
      );

      await tester.tap(find.byType(Checkbox));
      await tester.pump();
      expect(checked(tester), ['Gate code']);
      expect(find.text('1 selected'), findsOneWidget);
      // Nothing opened beside the list.
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsNothing);
      // Every row has its checkbox while anything is selected.
      expect(find.byType(Checkbox), findsNWidgets(names(tester).length));
      // What can be done with them, where the list's title was.
      for (final action in [
        'Clear the selection',
        'Select all',
        'Add to favorites',
        'Move',
        'Move to trash',
      ]) {
        expect(inBar(action), findsOneWidget);
      }

      // A plain click still opens an item, and the selection stays.
      await tester.tap(row('Everyday card'));
      await tester.pump();
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsOneWidget);
      expect(checked(tester), ['Gate code']);

      // Its checkbox again takes it out, and the bar goes with the last.
      await tester.tap(
        find.descendant(of: row('Gate code'), matching: find.byType(Checkbox)),
      );
      await tester.pump();
      expect(checked(tester), isEmpty);
      expect(find.byType(SelectionBar), findsNothing);
    },
  );

  testWidgets(
    'Shift and a click selects from the last one selected to that one, either way; Ctrl and a click adds one or takes it out, without opening it',
    (tester) async {
      await unlock(tester, computer);
      final all = names(tester);
      expect(all, hasLength(7));

      await clickWith(tester, ctrl, row(all[1]));
      expect(checked(tester), [all[1]]);
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsNothing);

      await clickWith(tester, shift, row(all[4]));
      expect(checked(tester), all.sublist(1, 5));
      expect(find.text('4 selected'), findsOneWidget);
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsNothing);

      // One out of the middle.
      await clickWith(tester, ctrl, row(all[2]));
      expect(checked(tester), [all[1], all[3], all[4]]);

      // A new start, then up the list from it.
      await clickWith(tester, ctrl, row(all[6]));
      await clickWith(tester, shift, row(all[5]));
      expect(checked(tester), [all[1], all[3], all[4], all[5], all[6]]);
      expect(find.text('5 selected'), findsOneWidget);

      // Shift on a checkbox does the same as on its row.
      await tester.tap(inBar('Clear the selection'));
      await tester.pump();
      await clickWith(tester, ctrl, row(all[0]));
      await clickWith(
        tester,
        shift,
        find.descendant(of: row(all[2]), matching: find.byType(Checkbox)),
      );
      expect(checked(tester), all.sublist(0, 3));
    },
  );

  testWidgets(
    'Ctrl+A selects everything shown once the list has the focus, Escape clears it, and a search keeps only what it still shows',
    (tester) async {
      await unlock(tester, computer);
      final all = names(tester);

      // In the search field, Ctrl+A is the field's own.
      await tester.tap(find.byType(TextField));
      await tester.sendKeyDownEvent(ctrl);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
      await tester.sendKeyUpEvent(ctrl);
      await tester.pump();
      expect(find.byType(SelectionBar), findsNothing);

      // A click on a row gives the list the focus.
      await tester.tap(row(all[0]));
      await tester.pump();
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

      // The bar's Select all is everything shown too: with a search, what
      // it found.
      await clickWith(tester, ctrl, row('Build server'));
      await tester.tap(inBar('Select all'));
      await tester.pump();
      expect(checked(tester), all);
      await tester.enterText(find.byType(TextField), 'server');
      await tester.pump();
      final found = names(tester);
      expect(found, contains('Build server'));
      expect(found.length, lessThan(all.length));
      expect(checked(tester), found);
      // The others aren't selected again when they're back.
      await tester.enterText(find.byType(TextField), '');
      await tester.pump();
      expect(names(tester), all);
      expect(checked(tester), found);

      // Another place starts with nothing selected.
      await tester.tap(find.text('Favorites'));
      await tester.pump();
      expect(find.byType(SelectionBar), findsNothing);
      expect(names(tester), ['Example Mail']);
    },
  );

  testWidgets('Command is the key on a Mac, Ctrl elsewhere', (tester) async {
    const meta = LogicalKeyboardKey.metaLeft;
    await tester.sendKeyDownEvent(ctrl);
    expect(commandHeld(), isTrue);
    await tester.sendKeyUpEvent(ctrl);
    await tester.sendKeyDownEvent(meta);
    expect(commandHeld(), isFalse);
    await tester.sendKeyUpEvent(meta);

    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    try {
      await tester.sendKeyDownEvent(meta, platform: 'macos');
      expect(commandHeld(), isTrue);
      await tester.sendKeyUpEvent(meta, platform: 'macos');
      await tester.sendKeyDownEvent(ctrl, platform: 'macos');
      expect(commandHeld(), isFalse);
      await tester.sendKeyUpEvent(ctrl, platform: 'macos');
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets(
    'in the narrowest computer window the selection\'s bar fits, with every action',
    (tester) async {
      await unlock(tester, const Size(900, 640));
      await clickWith(tester, ctrl, row('Example Mail'));
      await tester.tap(inBar('Select all'));
      await tester.pump();
      expect(checked(tester), hasLength(7));
      // The number alone, where "7 selected" has no room.
      expect(
        find.descendant(
          of: find.byType(SelectionBar),
          matching: find.text('7'),
        ),
        findsOneWidget,
      );
      for (final action in ['Add to favorites', 'Move', 'Move to trash']) {
        expect(inBar(action), findsOneWidget);
      }
      // Only favorites selected: the star takes them out.
      await tester.tap(inBar('Clear the selection'));
      await tester.pump();
      await clickWith(tester, ctrl, row('Example Mail'));
      expect(inBar('Remove from favorites'), findsOneWidget);
      expect(inBar('Add to favorites'), findsNothing);
    },
  );

  testWidgets(
    'the trash asks once, with how many, and says how many are left as they are and why; it isn\'t offered when none may be deleted',
    (tester) async {
      await unlock(tester, computer, changed: {officeWifi: noDelete});
      await clickWith(tester, ctrl, row('Office Wi-Fi'));
      // The one item the organization keeps: no trash for it.
      expect(inBar('Move to trash'), findsNothing);
      expect(inBar('Add to favorites'), findsOneWidget);

      await tester.tap(inBar('Select all'));
      await tester.pump();
      await tester.tap(inBar('Move to trash'));
      await tester.pumpAndSettle();
      expect(find.text('Move 6 items to the trash?'), findsOneWidget);
      expect(
        find.text(
          '1 item will be left as it is: your organization doesn\'t let you delete it.',
        ),
        findsOneWidget,
      );
      expect(
        find.text('You can restore them from the trash in the web vault.'),
        findsOneWidget,
      );
      // No master password is asked where no item asks for it.
      expect(inDialog(find.byType(TextField)), findsNothing);

      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(checked(tester), hasLength(7));
    },
  );

  testWidgets(
    'moving asks where: the person\'s folders for their own items, and what\'s left is said before it\'s done',
    (tester) async {
      await unlock(tester, computer);
      await clickWith(tester, ctrl, row('Gate code'));
      await clickWith(tester, ctrl, row('Office Wi-Fi'));
      await tester.tap(inBar('Move'));
      await tester.pumpAndSettle();
      expect(find.text('Move 2 items to'), findsOneWidget);
      // The person's own, then the organization that has one of them.
      expect(inDialog(find.text('Personal')), findsNWidgets(2));
      expect(inDialog(find.text('No folder')), findsOneWidget);
      expect(inDialog(find.text('Contoso')), findsOneWidget);
      expect(inDialog(find.text('IT')), findsOneWidget);

      // The folder (it's called Personal too, under the heading).
      await tester.tap(inDialog(find.text('Personal')).last);
      await tester.pumpAndSettle();
      expect(find.text('Move 1 item to Personal?'), findsOneWidget);
      expect(
        find.text(
          '1 item will be left as it is: it belongs to an organization, and stays in its shared folders.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(checked(tester), ['Gate code', 'Office Wi-Fi']);

      // In its shared folder, the one it's in isn't offered again.
      await tester.tap(inBar('Clear the selection'));
      await tester.pump();
      await tester.tap(
        find.descendant(of: find.byType(Sidebar), matching: find.text('IT')),
      );
      await tester.pump();
      expect(names(tester), ['Office Wi-Fi']);
      await clickWith(tester, ctrl, row('Office Wi-Fi'));
      await tester.tap(inBar('Move'));
      await tester.pumpAndSettle();
      expect(find.text('Move 1 item to'), findsOneWidget);
      expect(inDialog(find.text('Personal')), findsNothing);
      expect(inDialog(find.text('IT')), findsNothing);
      expect(
        find.text('There\'s no other shared folder to move to.'),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'items that ask for the master password are done only once it\'s typed, and a wrong one does nothing',
    (tester) async {
      await unlock(tester, computer, asking: {mail, backup});
      await clickWith(tester, ctrl, row('Example Mail'));
      await clickWith(tester, ctrl, row('Backup service'));
      // Only items that ask: nothing is done without it.
      await tester.tap(inBar('Move to trash'));
      await tester.pumpAndSettle();
      expect(find.text('Move 2 items to the trash?'), findsOneWidget);
      expect(
        find.text('These items ask for your master password.'),
        findsOneWidget,
      );
      final field = inDialog(find.widgetWithText(TextField, 'Master password'));
      expect(field, findsOneWidget);
      await tester.tap(inDialog(find.text('Move to trash')));
      await tester.pump();
      expect(find.text('Move 2 items to the trash?'), findsOneWidget);

      await tester.enterText(field, 'not the password');
      await tester.tap(inDialog(find.text('Move to trash')));
      await pumpUntil(tester, find.text('That master password isn\'t right.'));
      expect(find.text('Move 2 items to the trash?'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(checked(tester), ['Backup service', 'Example Mail']);

      // With others that don't ask: they can go without it, and these two
      // are left unless it's typed. A favorite asks too.
      await tester.tap(inBar('Select all'));
      await tester.pump();
      await tester.tap(inBar('Add to favorites'));
      await tester.pumpAndSettle();
      expect(find.text('Add 5 items to Favorites?'), findsOneWidget);
      expect(
        find.text(
          '2 more items ask for your master password. Enter it to include them, or leave it empty to leave them as they are.',
        ),
        findsOneWidget,
      );
      expect(field, findsOneWidget);
    },
  );

  testWidgets(
    'on a phone, a long press starts selecting, a tap then selects or takes out, and the top says how many, with Select all and a way out',
    (tester) async {
      await unlock(tester, phone);
      await tester.tap(find.text('Logins'));
      await tester.pumpAndSettle();
      expect(find.byType(Checkbox), findsNothing);

      await tester.longPress(phoneRow('Example Mail'));
      await tester.pumpAndSettle();
      expect(find.text('1 selected'), findsOneWidget);
      expect(phoneChecked(tester), ['Example Mail']);
      expect(find.byType(Checkbox), findsNWidgets(3));
      // The actions are where the tabs were, and New item is away.
      expect(find.byType(NavigationBar), findsNothing);
      expect(find.byTooltip('New item'), findsNothing);
      final bar = find.byType(PhoneSelectionBar);
      for (final action in ['Remove from favorites', 'Move', 'Move to trash']) {
        expect(
          find.descendant(of: bar, matching: find.text(action)),
          findsOneWidget,
        );
      }

      // A tap selects now: the item's page doesn't open.
      await tester.tap(phoneRow('Backup service'));
      await tester.pumpAndSettle();
      expect(find.text('2 selected'), findsOneWidget);
      expect(phoneChecked(tester), ['Backup service', 'Example Mail']);
      expect(find.byTooltip('Copy password'), findsNothing);
      expect(
        find.descendant(of: bar, matching: find.text('Add to favorites')),
        findsOneWidget,
      );
      await tester.tap(phoneRow('Backup service'));
      await tester.pumpAndSettle();
      expect(find.text('1 selected'), findsOneWidget);

      await tester.tap(find.text('Select all'));
      await tester.pumpAndSettle();
      expect(find.text('3 selected'), findsOneWidget);
      expect(phoneChecked(tester), hasLength(3));

      await tester.tap(find.byTooltip('Clear the selection'));
      await tester.pumpAndSettle();
      expect(phoneChecked(tester), isEmpty);
      expect(find.byType(Checkbox), findsNothing);
      expect(find.byType(NavigationBar), findsOneWidget);
      expect(find.byTooltip('New item'), findsOneWidget);
      // And a tap opens an item again.
      await tester.tap(phoneRow('Example Mail'));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Copy password'), findsOneWidget);
    },
  );

  testWidgets(
    'on a phone, going back clears the selection first, then leaves the list',
    (tester) async {
      await unlock(tester, phone);
      await tester.tap(find.text('Logins'));
      await tester.pumpAndSettle();
      await tester.longPress(phoneRow('Office Wi-Fi'));
      await tester.pumpAndSettle();
      await tester.tap(phoneRow('Example Mail'));
      await tester.pumpAndSettle();
      expect(find.text('2 selected'), findsOneWidget);

      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(find.textContaining('selected'), findsNothing);
      expect(phoneChecked(tester), isEmpty);
      // Still the logins.
      expect(phoneRow('Office Wi-Fi'), findsOneWidget);
      expect(find.byTooltip('Back to all items'), findsOneWidget);

      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(find.text('Favorites'), findsOneWidget);
    },
  );

  testWidgets(
    'on a phone, the trash asks with how many and what\'s left, and an item the organization keeps has no trash',
    (tester) async {
      await unlock(tester, phone, changed: {officeWifi: noDelete});
      await tester.tap(find.text('Logins'));
      await tester.pumpAndSettle();
      final bar = find.byType(PhoneSelectionBar);
      await tester.longPress(phoneRow('Office Wi-Fi'));
      await tester.pumpAndSettle();
      expect(
        find.descendant(of: bar, matching: find.text('Move to trash')),
        findsNothing,
      );
      await tester.tap(find.text('Select all'));
      await tester.pumpAndSettle();
      await tester.tap(
        find.descendant(of: bar, matching: find.text('Move to trash')),
      );
      await tester.pumpAndSettle();
      expect(find.text('Move 2 items to the trash?'), findsOneWidget);
      expect(
        find.text(
          '1 item will be left as it is: your organization doesn\'t let you delete it.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(find.text('3 selected'), findsOneWidget);
    },
  );

  for (final size in const [Size(360, 640), Size(320, 568)]) {
    testWidgets(
      'at ${size.width.toInt()} wide with the text 1.3 times its size, selecting on a phone fits',
      (tester) async {
        tester.platformDispatcher.textScaleFactorTestValue = 1.3;
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
        await unlock(
          tester,
          size,
          asking: {backup},
          changed: {officeWifi: noDelete},
        );
        await tester.tap(find.text('Logins'));
        await tester.pumpAndSettle();
        await tester.longPress(phoneRow('Example Mail'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Select all'));
        await tester.pumpAndSettle();
        expect(find.text('3 selected'), findsOneWidget);

        final bar = find.byType(PhoneSelectionBar);
        await tester.tap(find.descendant(of: bar, matching: find.text('Move')));
        await tester.pumpAndSettle();
        expect(find.text('Move 3 items to'), findsOneWidget);
        await tester.tap(find.text('Cancel'));
        await tester.pumpAndSettle();

        // The longest question: one left, one that asks, and the field.
        await tester.tap(
          find.descendant(of: bar, matching: find.text('Move to trash')),
        );
        await tester.pumpAndSettle();
        expect(find.text('Move 1 item to the trash?'), findsOneWidget);
        expect(
          find.widgetWithText(TextField, 'Master password'),
          findsOneWidget,
        );
        await tester.tap(find.text('Cancel'));
        await tester.pumpAndSettle();
      },
    );
  }

  testWidgets(
    'moving several items to the trash moves the ones the person may delete in one call, says how many went and what was left, and the list shows it',
    (tester) async {
      final standIn = await signIn(
        tester,
        first: (s) => s.item(officeWifi).addAll(noDelete),
      );
      await tester.tap(row('Example Mail'));
      await tester.pump();
      await clickWith(tester, ctrl, row('Example Mail'));
      await tester.tap(inBar('Select all'));
      await tester.pump();
      await tester.tap(inBar('Move to trash'));
      await tester.pumpAndSettle();
      expect(find.text('Move 6 items to the trash?'), findsOneWidget);
      await tester.tap(inDialog(find.text('Move to trash')));
      await pumpUntil(
        tester,
        find.text(
          'Moved 6 items to the trash. 1 was left as it is: your organization doesn\'t let you delete it.',
        ),
      );
      expect(find.text('Open trash'), findsOneWidget);
      // One call for all six, without the one that stays.
      expect(standIn.changes, ['PUT /api/ciphers/delete']);
      final ids =
          (standIn.sent['PUT /api/ciphers/delete'] as Map)['ids'] as List;
      expect(ids, hasLength(6));
      expect(ids, isNot(contains(officeWifi)));
      // The list has what's left, nothing is selected, and the item that
      // was open beside it is closed.
      expect(names(tester), ['Office Wi-Fi']);
      expect(find.byType(SelectionBar), findsNothing);
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsNothing);
    },
  );

  testWidgets(
    'favorites and a folder for several items: no question when all can be done, and an organization\'s item stays in its shared folder, said before and after',
    (tester) async {
      final standIn = await signIn(tester);
      await clickWith(tester, ctrl, row('Gate code'));
      await clickWith(tester, ctrl, row('Everyday card'));
      await clickWith(tester, ctrl, row('Office Wi-Fi'));

      // Favorites are the person's own mark: every item takes it.
      await tester.tap(inBar('Add to favorites'));
      await pumpUntil(tester, find.text('Added 3 items to Favorites.'));
      expect(standIn.changes, hasLength(3));
      expect(standIn.changes.toSet(), {
        for (final id in [
          '60718293-a4b5-4c63-def4-05162738495a',
          '718293a4-b5c6-4d74-8f01-162738495a6b',
          officeWifi,
        ])
          'PUT /api/ciphers/$id/partial',
      });
      expect(standIn.item(officeWifi)['favorite'], isTrue);
      expect(find.byType(SelectionBar), findsNothing);
      await tester.tap(find.text('Favorites'));
      await tester.pump();
      expect(names(tester), hasLength(4));

      // All four to the folder: Example Mail is in it already, and Office
      // Wi-Fi is the organization's.
      standIn.asked.clear();
      await clickWith(tester, ctrl, row('Gate code'));
      await tester.tap(inBar('Select all'));
      await tester.pump();
      expect(inBar('Remove from favorites'), findsOneWidget);
      await tester.tap(inBar('Move'));
      await tester.pumpAndSettle();
      await tester.tap(inDialog(find.text('Personal')).last);
      await tester.pumpAndSettle();
      expect(find.text('Move 3 items to Personal?'), findsOneWidget);
      await tester.tap(inDialog(find.text('Move')));
      await pumpUntil(
        tester,
        find.text(
          'Moved 3 items to Personal. 1 was left as it is: it belongs to an organization, and stays in its shared folders.',
        ),
      );
      // One call, for the two that weren't there yet.
      expect(standIn.changes, ['PUT /api/ciphers/move']);
      final sent = standIn.sent['PUT /api/ciphers/move'] as Map;
      expect(sent['ids'], hasLength(2));
      expect(sent['folderId'], '1b2c3d4e-5f60-4718-89a0-b1c2d3e4f506');
      expect(standIn.item(officeWifi)['folderId'], isNull);
    },
  );

  testWidgets(
    'when the server refuses one item, what went and what didn\'t is said in a dialog, and the one that didn\'t stays selected',
    (tester) async {
      final standIn = await signIn(tester);
      standIn.refuses.add(officeWifi);
      await clickWith(tester, ctrl, row('Gate code'));
      await clickWith(tester, ctrl, row('Everyday card'));
      await clickWith(tester, ctrl, row('Office Wi-Fi'));
      await tester.tap(inBar('Add to favorites'));
      await pumpUntil(tester, find.text('1 item wasn\'t added to Favorites'));
      await tester.pumpAndSettle();
      expect(
        find.text(
          'Added 2 items to Favorites. What didn\'t go is still selected, to try again.',
        ),
        findsOneWidget,
      );
      // With what the server said of it, as for one item.
      expect(
        inDialog(
          find.text('Your server said: Cipher is not write accessible.'),
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('OK'));
      await tester.pumpAndSettle();
      expect(checked(tester), ['Office Wi-Fi']);
      expect(find.text('1 selected'), findsOneWidget);
    },
  );

  testWidgets(
    'the master password, typed once, includes every selected item that asks for it; left empty, they\'re left and said',
    (tester) async {
      final standIn = await signIn(
        tester,
        first: (s) {
          s.item(mail)['reprompt'] = 1;
          s.item(backup)['reprompt'] = 1;
        },
      );
      await clickWith(tester, ctrl, row('Example Mail'));
      await clickWith(tester, ctrl, row('Backup service'));
      await clickWith(tester, ctrl, row('Gate code'));
      await tester.tap(inBar('Move to trash'));
      await tester.pumpAndSettle();
      expect(find.text('Move 1 item to the trash?'), findsOneWidget);
      // Left empty: the one that doesn't ask goes.
      await tester.tap(inDialog(find.text('Move to trash')));
      await pumpUntil(
        tester,
        find.text(
          'Moved 1 item to the trash. 2 were left as they are: they ask for your master password.',
        ),
      );
      expect((standIn.sent['PUT /api/ciphers/delete'] as Map)['ids'], [
        '60718293-a4b5-4c63-def4-05162738495a',
      ]);
      expect(names(tester), hasLength(6));

      // A wrong one: nothing is sent. Then the right one, for both.
      standIn.asked.clear();
      await clickWith(tester, ctrl, row('Example Mail'));
      await clickWith(tester, ctrl, row('Backup service'));
      await tester.tap(inBar('Move to trash'));
      await tester.pumpAndSettle();
      final field = inDialog(find.widgetWithText(TextField, 'Master password'));
      await tester.enterText(field, 'not the password');
      await tester.tap(inDialog(find.text('Move to trash')));
      await pumpUntil(tester, find.text('That master password isn\'t right.'));
      expect(standIn.changes, isEmpty);
      await tester.enterText(field, samplePassword);
      await tester.tap(inDialog(find.text('Move to trash')));
      await pumpUntil(tester, find.text('Moved 2 items to the trash.'));
      expect(standIn.changes, ['PUT /api/ciphers/delete']);
      expect(
        (standIn.sent['PUT /api/ciphers/delete'] as Map)['ids'],
        hasLength(2),
      );
      expect(names(tester), hasLength(4));
    },
  );

  testWidgets(
    'on a phone, what happened is said the same, and the list is as it was before selecting',
    (tester) async {
      final standIn = await signIn(tester);
      // The same window, as narrow as a phone.
      tester.view.physicalSize = phone;
      await tester.pumpAndSettle();
      await tester.tap(find.text('Logins'));
      await tester.pumpAndSettle();
      await tester.longPress(phoneRow('Backup service'));
      await tester.pumpAndSettle();
      await tester.tap(phoneRow('Office Wi-Fi'));
      await tester.pumpAndSettle();
      await tester.tap(
        find.descendant(
          of: find.byType(PhoneSelectionBar),
          matching: find.text('Move to trash'),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Move 2 items to the trash?'), findsOneWidget);
      await tester.tap(inDialog(find.text('Move to trash')));
      await pumpUntil(tester, find.text('Moved 2 items to the trash.'));
      expect(standIn.changes, ['PUT /api/ciphers/delete']);
      expect(find.byType(PhoneSelectionBar), findsNothing);
      expect(find.byType(NavigationBar), findsOneWidget);
      expect(phoneRow('Example Mail'), findsOneWidget);
      expect(phoneRow('Backup service'), findsNothing);
      expect(phoneRow('Office Wi-Fi'), findsNothing);
    },
  );
}
