// Export and import in the app (DESIGN.md in the meta repo, "Archive,
// export, and import in the apps"; transfer.dart): Export's formats, the
// master password first, and the file the core writes where the save dialog
// said; Import's formats, a password-protected file's password, what came in
// before anything is saved, and where it may go. Against the real core, with
// the device's save dialog and picker stood in for, and for what the server
// was sent, a stand-in for it on this computer.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/vault.dart';

import 'support.dart';

/// A stand-in for PassBundle's server: the sample vault's account, with
/// items imported as the server's calls for them do. [asked] has each call,
/// and [sent] the body of each.
class StandIn {
  StandIn._(this._server);

  final HttpServer _server;
  final asked = <String>[];
  final sent = <String, List<Map<String, dynamic>>>{};
  final Map<String, dynamic> sync = jsonDecode(
    File('test/sample/sync.json').readAsStringSync(),
  ) as Map<String, dynamic>;
  var _made = 0;

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
    final text = await utf8.decodeStream(request);
    final body = text.isEmpty ? null : jsonDecode(text);
    if (body is Map<String, dynamic>) {
      sent.putIfAbsent(path, () => []).add(body);
    }
    void json(int status, Object? value) {
      request.response.statusCode = status;
      request.response.headers.contentType = ContentType.json;
      if (value != null) request.response.write(jsonEncode(value));
    }

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
      case '/api/folders':
        final id = 'f0000000-0000-4000-9000-${'${_made++}'.padLeft(12, '0')}';
        (sync['folders'] as List).add({'id': id, 'name': body['name']});
        json(200, {'id': id, 'name': body['name'], 'object': 'folder'});
      case '/api/ciphers/import':
        final made = (body as Map<String, dynamic>)['ciphers'] as List;
        for (final c in made.cast<Map<String, dynamic>>()) {
          ciphers.add({
            ...c,
            'id': '00000000-0000-4000-9000-${'${_made++}'.padLeft(12, '0')}',
            'collectionIds': <String>[],
            'edit': true,
            'viewPassword': true,
          });
        }
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

/// The device's own dialogs, stood in for: the save dialog answers [saveTo]
/// (null: the person gave up), and the picker each of [picks] in turn.
class Dialogs {
  Dialogs(WidgetTester tester) {
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(Core.platform, (call) async {
      switch (call.method) {
        case 'saveFile':
          suggested.add((call.arguments as Map)['name'] as String);
          return saveTo;
        case 'pickFile':
          pickedWith.add(Map<String, dynamic>.from(call.arguments as Map));
          return picks.isEmpty ? null : picks.removeAt(0);
        default:
          return null;
      }
    });
    addTearDown(() => messenger.setMockMethodCallHandler(Core.platform, null));
  }

  String? saveTo;
  final suggested = <String>[];
  final picks = <Map<String, Object?>?>[];
  final pickedWith = <Map<String, dynamic>>[];
}

/// A folder of the test's own, deleted after it.
Directory scratch() {
  final dir = Directory.systemTemp.createTempSync('pb-app-transfer-');
  addTearDown(() => dir.deleteSync(recursive: true));
  return dir;
}

/// A file for the picker to answer.
Map<String, Object?> fileToPick(String name, String contents) {
  final file = File('${scratch().path}/$name')..writeAsStringSync(contents);
  return {'path': file.path, 'name': name};
}

void main() {
  setUpAll(loadFonts);

  const computer = Size(1280, 1100);

  Future<void> unlockHere(WidgetTester tester) async {
    setSize(tester, computer);
    await tester.pumpWidget(const PassBundleApp());
    await tester.enterText(find.byType(TextField), samplePassword);
    await tester.tap(find.text('Unlock'));
    await pumpUntil(tester, find.text('Example Mail'));
  }

  /// Signed in to a stand-in for the server, on a computer.
  Future<StandIn> signIn(WidgetTester tester) async {
    final standIn = await StandIn.start(tester);
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
    await pumpUntil(tester, find.byTooltip('Sync'));
    // The offer to import from this computer's browsers, if it comes.
    if (find.text('Not now').evaluate().isNotEmpty) {
      await tester.tap(find.text('Not now'));
      await tester.pumpAndSettle();
    }
    return standIn;
  }

  testWidgets(
    'Export asks for the master password first, then where, and the core writes the file there',
    (tester) async {
      final dialogs = Dialogs(tester);
      final dir = scratch();
      openSample();
      await unlockHere(tester);
      await tester.tap(await menuEntry(tester, 'Export'));
      await tester.pumpAndSettle();
      // The person's own items, and the organization's said in one line.
      expect(
        find.text(
          'Exports the 6 items in Personal. An organization\'s items are exported in the web vault.',
        ),
        findsOneWidget,
      );
      expect(
        find.textContaining('holds your passwords in plain text'),
        findsOneWidget,
      );

      // Not without the master password, and not with a wrong one.
      await tester.tap(find.byKey(const ValueKey('export-go')));
      await tester.pump();
      expect(
        find.text('Enter your master password to export.'),
        findsOneWidget,
      );
      await tester.enterText(
        find.byKey(const ValueKey('export-master-password')),
        'not the master password',
      );
      await tester.tap(find.byKey(const ValueKey('export-go')));
      await pumpUntil(tester, find.text('That master password isn\'t right.'));
      expect(dialogs.suggested, isEmpty);

      // Right: then the save dialog, and the file where it said.
      final path = '${dir.path}/export.json';
      dialogs.saveTo = path;
      await tester.enterText(
        find.byKey(const ValueKey('export-master-password')),
        samplePassword,
      );
      await tester.tap(find.byKey(const ValueKey('export-go')));
      await pumpUntil(tester, find.text('Exported 6 items to export.json.'));
      expect(dialogs.suggested.single, startsWith('passbundle_export_'));
      expect(dialogs.suggested.single, endsWith('.json'));
      final written = jsonDecode(File(path).readAsStringSync()) as Map;
      expect(written['encrypted'], isFalse);
      expect(written['items'], hasLength(6));

      // Password-protected: the file password typed twice, the same.
      await tester.tap(await menuEntry(tester, 'Export'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('format-encrypted_json')));
      await tester.pump();
      expect(find.textContaining('plain text'), findsNothing);
      await tester.enterText(
        find.byKey(const ValueKey('file-password')),
        'made-up file password',
      );
      await tester.enterText(
        find.byKey(const ValueKey('file-password-again')),
        'not the same',
      );
      await tester.enterText(
        find.byKey(const ValueKey('export-master-password')),
        samplePassword,
      );
      await tester.tap(find.byKey(const ValueKey('export-go')));
      await tester.pump();
      expect(find.text('The two passwords aren\'t the same.'), findsOneWidget);
      await tester.enterText(
        find.byKey(const ValueKey('file-password-again')),
        'made-up file password',
      );
      final protected = '${dir.path}/protected.json';
      dialogs.saveTo = protected;
      await tester.tap(find.byKey(const ValueKey('export-go')));
      await pumpUntil(tester, find.text('Exported 6 items to protected.json.'));
      expect(
        dialogs.suggested.last,
        startsWith('passbundle_encrypted_export_'),
      );
      final file = File(protected).readAsStringSync();
      final doc = jsonDecode(file) as Map;
      expect(doc['passwordProtected'], isTrue);
      expect(doc['kdfIterations'], 600000);
      expect(file, isNot(contains('not-a-real-password')));
    },
  );

  testWidgets(
    'a browser\'s export: its steps, what came in and what didn\'t before anything is saved, then saved encrypted',
    (tester) async {
      final standIn = await signIn(tester);
      final dialogs = Dialogs(tester)
        ..picks.add(
          fileToPick(
            'Chrome Passwords.csv',
            'name,url,username,password,note\n'
                'mail.example.test,https://mail.example.test/login,ana.lopez@example.test,not-a-real-password-1,\n'
                'wiki.example.test,https://wiki.example.test/,ana,made-up-wiki-password,\n'
                'shop.example.test,https://shop.example.test/,ana,,\n',
          ),
        );
      await tester.tap(await menuEntry(tester, 'Import'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('import-format-chrome')));
      await tester.pump();
      // Chrome's own Export, step by step.
      expect(find.text('Export from Google Chrome'), findsOneWidget);
      expect(
        find.text('Next to Export passwords, choose Download file.'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const ValueKey('import-choose-file')));
      await pumpUntil(tester, find.byKey(const ValueKey('import-go')));
      expect(dialogs.pickedWith.single['import'], isTrue);
      expect(find.text('1 item comes in: 1 login.'), findsOneWidget);
      expect(
        find.text(
          '1 is already in your vault, or twice in the file, so it\'s left out.',
        ),
        findsOneWidget,
      );
      expect(
        find.text('1 saved sign-in has no password, so it\'s skipped.'),
        findsOneWidget,
      );
      // Nothing sent yet.
      expect(standIn.asked, isNot(contains('POST /api/ciphers/import')));

      await tester.tap(find.byKey(const ValueKey('import-go')));
      await pumpUntil(tester, find.byKey(const ValueKey('import-done')));
      expect(find.text('1 item came in.'), findsOneWidget);
      expect(
        find.textContaining(
          'Chrome Passwords.csv has every password in plain text.',
        ),
        findsOneWidget,
      );
      final body = standIn.sent['/api/ciphers/import']!.single;
      expect(jsonEncode(body), isNot(contains('made-up-wiki-password')));
      expect(jsonEncode(body), isNot(contains('wiki.example.test')));
      await tester.tap(find.text('Done'));
      await tester.pumpAndSettle();
      // In the vault now.
      expect(find.widgetWithText(ItemRow, 'wiki.example.test'), findsOneWidget);
    },
  );

  testWidgets(
    'a password-protected export asks for its password, and says so when it\'s wrong',
    (tester) async {
      final vectors = jsonDecode(
        File('../core/tests/vectors/vault-export-v1.json').readAsStringSync(),
      ) as Map<String, dynamic>;
      final dialogs = Dialogs(tester)
        ..picks.add(
          fileToPick(
            'protected.json',
            (vectors['pbkdf2'] as Map)['file'] as String,
          ),
        );
      openSample();
      await unlockHere(tester);
      await tester.tap(await menuEntry(tester, 'Import'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('import-choose-file')));
      await pumpUntil(tester, find.text('This file has a password'));
      expect(dialogs.pickedWith, hasLength(1));
      await tester.enterText(
        find.byKey(const ValueKey('import-file-password')),
        'not the file password',
      );
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.text('That isn\'t this file\'s password.'));
      await tester.enterText(
        find.byKey(const ValueKey('import-file-password')),
        vectors['password'] as String,
      );
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.byKey(const ValueKey('import-go')));
      expect(
        find.text('3 items come in: 1 login, 1 note, 1 card.'),
        findsOneWidget,
      );
      expect(find.text('1 is archived, and stays in Archive.'), findsOneWidget);
      expect(find.byKey(const ValueKey('import-to-personal')), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('import-go')), findsNothing);
    },
  );

  testWidgets(
    'where an organization keeps items out of Personal, the import goes to a shared folder the person may add to',
    (tester) async {
      final dir = openSample(
        policies: [
          {
            'organizationId': '8f9a7c1e-3b2d-4e5f-9a8b-7c6d5e4f3a2b',
            'type': 5,
            'enabled': true,
          },
        ],
      );
      // A member of Contoso, not its owner, whom the policy binds: in the
      // account's own folder, where the core keeps the sample once it's open.
      final syncFile = Directory('$dir/accounts')
          .listSync()
          .whereType<Directory>()
          .map((d) => File('${d.path}/sync.json'))
          .firstWhere((f) => f.existsSync());
      final sync = jsonDecode(syncFile.readAsStringSync()) as Map;
      final memberships = (sync['profile'] as Map)['organizations'] as List;
      (memberships.first as Map)['type'] = 2;
      syncFile.writeAsStringSync(jsonEncode(sync));
      Dialogs(tester).picks.add(
        fileToPick(
          'export.json',
          jsonEncode({
            'encrypted': false,
            'folders': [],
            'items': [
              {
                'type': 1,
                'name': 'Made-up login',
                'login': {'username': 'ana', 'password': 'made-up-password'},
              },
            ],
          }),
        ),
      );
      await unlockHere(tester);
      await tester.tap(await menuEntry(tester, 'Import'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('import-choose-file')));
      await pumpUntil(tester, find.byKey(const ValueKey('import-go')));
      expect(find.byKey(const ValueKey('import-to-personal')), findsNothing);
      expect(find.text('Contoso › IT'), findsOneWidget);
      expect(find.text('Contoso keeps items out of Personal.'), findsOneWidget);
    },
  );
}
