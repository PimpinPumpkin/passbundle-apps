// Archive in the app (DESIGN.md in the meta repo, "Archive, export, and
// import in the apps"): an item's Archive and Move out of Archive, and the
// Archive view beside the trash, on a computer and a phone. Against the real
// core, and for what the server was asked, a stand-in for it on this
// computer.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/main.dart';
import 'package:passbundle/vault.dart';

import 'support.dart';

// The sample vault's items: a login of the person's own.
const mail = '3d4e5f60-7182-4930-abc1-d2e3f4051627';

/// A stand-in for PassBundle's server: the sample vault's account, with
/// items archived and unarchived as the server's calls for them do.
/// [asked] has each call.
class StandIn {
  StandIn._(this._server);

  final HttpServer _server;
  final asked = <String>[];
  final Map<String, dynamic> sync = jsonDecode(
    File('test/sample/sync.json').readAsStringSync(),
  ) as Map<String, dynamic>;

  String get url => 'http://127.0.0.1:${_server.port}';

  List<Map<String, dynamic>> get ciphers =>
      (sync['ciphers'] as List).cast<Map<String, dynamic>>();

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
    asked.add('${request.method} $path');
    await utf8.decodeStream(request);
    void json(int status, Object? value) {
      request.response.statusCode = status;
      request.response.headers.contentType = ContentType.json;
      if (value != null) request.response.write(jsonEncode(value));
    }

    final archive = RegExp(r'^/api/ciphers/([^/]+)/(archive|unarchive)$')
        .firstMatch(path);
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
      case _ when archive != null:
        final cipher = ciphers.firstWhere((c) => c['id'] == archive.group(1));
        cipher['archivedDate'] = archive.group(2) == 'archive'
            ? '2026-11-16T09:00:00.000000Z'
            : null;
        json(200, cipher);
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

  const computer = Size(1280, 1100);

  /// Signed in to a stand-in for the server, at [size].
  Future<StandIn> signIn(WidgetTester tester, {Size size = computer}) async {
    final standIn = await StandIn.start(tester);
    openEmpty();
    setSize(tester, size);
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
    await pumpUntil(tester, find.byTooltip('Sync'));
    return standIn;
  }

  testWidgets(
    'Archive takes an item out of the list into Archive, beside the trash, and Move out of Archive brings it back',
    (tester) async {
      final standIn = await signIn(tester);
      await tester.tap(find.widgetWithText(ItemRow, 'Example Mail'));
      await tester.pump();
      expect(find.byTooltip('Archive'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('archive-item')));
      await pumpUntil(tester, find.text('Moved Example Mail to Archive.'));
      expect(standIn.asked, contains('PUT /api/ciphers/$mail/archive'));
      // Out of All items.
      expect(find.widgetWithText(ItemRow, 'Example Mail'), findsNothing);

      // In Archive, after the folders and before the trash.
      final archive = await menuEntry(tester, 'Archive');
      final trash = await menuEntry(tester, 'Trash');
      expect(
        tester.getTopLeft(archive).dy,
        lessThan(tester.getTopLeft(trash).dy),
      );
      await tester.tap(archive);
      await tester.pump();
      expect(find.widgetWithText(ItemRow, 'Example Mail'), findsOneWidget);
      expect(find.widgetWithText(ItemRow, 'Gate code'), findsNothing);
      await tester.tap(find.widgetWithText(ItemRow, 'Example Mail'));
      await tester.pump();
      expect(find.byTooltip('Move out of Archive'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('archive-item')));
      await pumpUntil(tester, find.text('Moved Example Mail out of Archive.'));
      expect(standIn.asked, contains('PUT /api/ciphers/$mail/unarchive'));
      expect(find.widgetWithText(ItemRow, 'Example Mail'), findsNothing);
      expect(
        find.text(
          'Nothing in Archive. An archived item stays in your vault, out of the lists, search, and filling.',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'on a phone, Archive is a row beside the trash, and an item archived from its page leaves the list',
    (tester) async {
      PhoneStandIn(tester, status: 'unavailable');
      final standIn = await signIn(tester, size: const Size(390, 844));
      expect(find.byKey(const ValueKey('archive')), findsOneWidget);
      await tester.tap(find.text('Example Mail').first);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('archive-item')));
      await pumpUntil(tester, find.text('Moved Example Mail to Archive.'));
      expect(standIn.asked, contains('PUT /api/ciphers/$mail/archive'));
      // Back on the list, without it, and in Archive.
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('archive-item')), findsNothing);
      await tester.ensureVisible(find.byKey(const ValueKey('archive')));
      await tester.tap(find.byKey(const ValueKey('archive')));
      await tester.pumpAndSettle();
      expect(find.text('Example Mail'), findsOneWidget);
      expect(find.text('Gate code'), findsNothing);
    },
  );
}
