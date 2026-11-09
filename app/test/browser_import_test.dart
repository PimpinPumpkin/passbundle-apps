// Importing from this computer's browsers (browser_import.dart), through the
// real core (PB_CORE_LIB, built with its test hooks) and made-up browser
// profiles with known fake passwords in a throwaway folder (the core's
// pb_test_browsers), never the real ones: a core with test hooks looks at no
// others. Signed in to a stand-in for the server on this computer that
// keeps what it's sent, so every request can be checked for plain text.
//
// The made-up computer (core/src/browser_import.rs, `make_sample`): Chrome's
// default profile "Person 1" (signed in to nothing), "Work" (signed in with
// the account's email), and "Home" (another account), and Firefox's default
// profile, with a Primary Password the core made up for the test.
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';

import 'support.dart';

const email = 'ana.lopez@example.test';

/// The stand-in server, in an isolate of its own: signing in (the sample
/// vault's keys), the sample vault's sync with what's been imported, and new
/// folders and imports, each request logged to the file it's given.
Future<void> _standIn(List<Object> args) async {
  final (port, log) = (args[0] as SendPort, File(args[1] as String));
  final sync = jsonDecode(
    File('test/sample/sync.json').readAsStringSync(),
  ) as Map<String, dynamic>;
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  port.send(server.port);
  var made = 0;
  await for (final request in server) {
    final body = await utf8.decoder.bind(request).join();
    final path = request.uri.path;
    log.writeAsStringSync(
      '${request.method} $path $body\n',
      mode: FileMode.append,
      flush: true,
    );
    final response = request.response..headers.contentType = ContentType.json;
    if (path == '/identity/accounts/prelogin') {
      response.write(jsonEncode({'kdf': 0, 'kdfIterations': 600000}));
    } else if (path == '/identity/connect/token') {
      response.write(
        jsonEncode({
          'access_token': 'a.b.c',
          'refresh_token': 'refresh',
          'expires_in': 3600,
          'token_type': 'Bearer',
          'Key': sync['profile']['key'],
          'PrivateKey': sync['profile']['privateKey'],
          'Kdf': 0,
          'KdfIterations': 600000,
        }),
      );
    } else if (path == '/api/sync') {
      response.write(jsonEncode(sync));
    } else if (path == '/api/folders' && request.method == 'POST') {
      made++;
      final folder = {
        'id': 'f0000000-0000-4000-8000-${'$made'.padLeft(12, '0')}',
        'name': (jsonDecode(body) as Map)['name'],
        'object': 'folder',
        'revisionDate': '2026-11-09T10:00:00.000Z',
      };
      (sync['folders'] as List).add(folder);
      response.write(jsonEncode(folder));
    } else if (path == '/api/ciphers/import') {
      final import = jsonDecode(body) as Map<String, dynamic>;
      final folderOf = {
        for (final r in (import['folderRelationships'] as List).cast<Map>())
          r['key'] as int: r['value'] as int,
      };
      final ciphers = (import['ciphers'] as List).cast<Map<String, dynamic>>();
      for (var i = 0; i < ciphers.length; i++) {
        made++;
        final folder = folderOf[i];
        (sync['ciphers'] as List).add({
          ...ciphers[i],
          'id': 'c0000000-0000-4000-8000-${'$made'.padLeft(12, '0')}',
          'object': 'cipherDetails',
          'edit': true,
          'viewPassword': true,
          'revisionDate': '2026-11-09T10:00:00.000Z',
          'folderId': folder == null
              ? null
              : (import['folders'] as List)[folder]['id'],
        });
      }
    } else {
      response.statusCode = 404;
    }
    await response.close();
  }
}

/// Signs in to the stand-in with the sample vault's email and master
/// password; answers what it was sent so far, as one text.
Future<String Function()> signIn(WidgetTester tester) async {
  final log = File(
    '${Directory.systemTemp.createTempSync('pb-browser-import-').path}/sent.txt',
  );
  final (standIn, port) = (await tester.runAsync(() async {
    final ports = ReceivePort();
    final standIn = await Isolate.spawn(_standIn, [ports.sendPort, log.path]);
    return (standIn, await ports.first as int);
  }))!;
  addTearDown(standIn.kill);
  await tester.pumpWidget(const PassBundleApp());
  await tester.tap(find.text('Self-hosted?'));
  await tester.pump();
  await tester.enterText(
    find.widgetWithText(TextField, 'Server'),
    'http://127.0.0.1:$port',
  );
  await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
  await tester.enterText(
    find.widgetWithText(TextField, 'Master password'),
    samplePassword,
  );
  await tester.tap(find.text('Sign in'));
  return () => log.existsSync() ? log.readAsStringSync() : '';
}

/// Whether a profile's box is checked, by its name.
bool checked(WidgetTester tester, String name) => tester
    .widget<CheckboxListTile>(
      find.ancestor(
        of: find.text(name),
        matching: find.byType(CheckboxListTile),
      ),
    )
    .value!;

void main() {
  setUpAll(loadFonts);
  late String root;
  setUp(() {
    root = Directory.systemTemp.createTempSync('pb-browsers-').path;
  });
  tearDown(() {
    Core().testBrowsers('');
    Directory(root).deleteSync(recursive: true);
  });

  testWidgets(
    'offered once after signing in: the profiles the rules choose, a Primary Password used once, everything encrypted before it\'s sent, and password saving turned off',
    (tester) async {
      tester.view.physicalSize = const Size(1280, 1000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      openEmpty();
      final core = Core();
      final primary = core.testBrowsers(root, platform: 'linux', email: email);
      final sent = await signIn(tester);

      await pumpUntil(tester, find.text('Import from your browsers'));
      // Then the profiles, found away from the screen.
      await pumpUntil(tester, find.text('Not now'));
      // Every profile, which is which, and why it's chosen or not.
      expect(find.text('Google Chrome'), findsOneWidget);
      expect(find.text('Firefox'), findsOneWidget);
      expect(find.textContaining('Signed in as $email'), findsOneWidget);
      expect(
        find.textContaining('Chosen: your PassBundle email'),
        findsOneWidget,
      );
      expect(
        find.textContaining('Signed in as ana.home@mail.example.test'),
        findsOneWidget,
      );
      expect(
        find.textContaining('Another account, so it\'s not chosen'),
        findsOneWidget,
      );
      expect(
        find.textContaining('Chosen: the default profile'),
        findsOneWidget,
      );
      expect(checked(tester, 'Work'), isTrue);
      expect(checked(tester, 'Person 1'), isFalse);
      expect(checked(tester, 'Home'), isFalse);
      expect(checked(tester, 'default-release'), isTrue);
      // Nothing is read before Import.
      expect(core.testBrowsersRead(), isEmpty);

      await tester.tap(find.widgetWithText(FilledButton, 'Import'));
      // Chrome's Work profile is read; Firefox's asks for its Primary Password.
      await pumpUntil(tester, find.text('Firefox\'s Primary Password'));
      await tester.enterText(
        find.byKey(const ValueKey('primary password')),
        'not-the-primary-password',
      );
      await tester.tap(find.text('Continue'));
      await pumpUntil(
        tester,
        find.text('That isn\'t this profile\'s Primary Password.'),
      );
      await tester.enterText(
        find.byKey(const ValueKey('primary password')),
        primary,
      );
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.text('Imported from your browsers'));

      // The Work profile's mail login has another password than the vault's
      // "Example Mail", so it's in the folder to compare.
      expect(find.text('2 new passwords are in your vault.'), findsOneWidget);
      expect(
        find.text(
          '1 has a different password from one you already have, so it\'s in "Check these: from Google Chrome" for you to compare.',
        ),
        findsOneWidget,
      );
      expect(
        find.textContaining('Password saving is off now.'),
        findsNWidgets(2),
      );

      // Only the chosen profiles were read (Firefox's each time it was
      // asked: without, with the wrong, and with the right Primary
      // Password), and none of what they hold went to the server as it is;
      // nor did the Primary Password.
      final read = core.testBrowsersRead();
      expect(read, hasLength(4), reason: '$read');
      expect(read.where((id) => id.endsWith('/Profile 1')), hasLength(1));
      expect(read.where((id) => id.startsWith('firefox:')), hasLength(3));
      final everything = sent();
      expect(everything, contains('POST /api/ciphers/import'));
      for (final plain in [
        'work-mail-password-2',
        'work-crm-password-3',
        'firefox-password-5',
        'personal-password-4',
        'person-one-password-1',
        'crm.example.test',
        'Check these',
        primary,
      ]) {
        expect(everything, isNot(contains(plain)), reason: plain);
      }

      await tester.tap(find.text('Done'));
      await pumpUntil(tester, find.text('crm.example.test'));
      expect(find.text('wiki.example.test'), findsOneWidget);

      // Offered once: locked and unlocked again, it isn't; it's in the menu.
      await tester.tap(find.text('Lock'));
      await tester.pump();
      expect(find.text('Unlock PassBundle'), findsOneWidget);
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Import from browsers'));
      expect(await tester.runAsync(Core.browserImportOffer), isFalse);
      expect(find.text('Import from your browsers'), findsNothing);
      await tester.tap(find.text('Import from browsers'));
      await pumpUntil(tester, find.text('Import from your browsers'));
      // Then the profiles, found away from the screen.
      await pumpUntil(tester, find.text('Not now'));
      await tester.tap(find.text('Not now'));
      await tester.pumpAndSettle();
      expect(core.testBrowsersRead(), isEmpty);
    },
  );

  testWidgets(
    'Not now reads nothing, and a browser that keeps its passwords to itself goes to its own Export, said in plain words',
    (tester) async {
      tester.view.physicalSize = const Size(1280, 1000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      openEmpty();
      final core = Core();
      // A Windows computer's layout: Edge's passwords there only Edge opens.
      core.testBrowsers(root, platform: 'windows', email: email);
      await signIn(tester);
      await pumpUntil(tester, find.text('Import from your browsers'));
      // Then the profiles, found away from the screen.
      await pumpUntil(tester, find.text('Not now'));
      await tester.tap(find.text('Not now'));
      await tester.pumpAndSettle();
      expect(core.testBrowsersRead(), isEmpty);

      await tester.tap(find.text('Import from browsers'));
      await pumpUntil(tester, find.text('Import from your browsers'));
      // Then the profiles, found away from the screen.
      await pumpUntil(tester, find.text('Not now'));
      expect(find.text('Microsoft Edge'), findsOneWidget);
      // Only Edge's default profile: nothing else to read.
      for (final name in ['Work', 'default-release']) {
        await tester.tap(find.text(name));
      }
      await tester.pump();
      await tester.tap(find.widgetWithText(FilledButton, 'Import'));
      await pumpUntil(tester, find.text('Imported from your browsers'));
      expect(
        find.text(
          'Edge protects its passwords on this computer so that only Edge can read them, and PassBundle doesn\'t get around that. Export them from Edge instead.',
        ),
        findsOneWidget,
      );
      expect(find.text('Import with its own Export'), findsOneWidget);
      expect(
        find.text('No new passwords: everything was already in your vault.'),
        findsOneWidget,
      );
    },
  );
}
