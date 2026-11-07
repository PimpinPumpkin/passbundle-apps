// What the app says when something goes wrong (docs/core.md, "What the apps
// say when something goes wrong"), against the real core and a stand-in for
// the server on this computer: a server address that isn't a PassBundle
// server is said under the server field; a wrong email or master password is
// said the same either way; offline, the vault says it shows this device's
// copy and saving says it didn't; and a save whose sync fails is saved once.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/main.dart';

import 'support.dart';

/// A stand-in for PassBundle's server: the sample vault's account
/// (test/sample), whose master password is [samplePassword]. It signs in
/// anyone unless [refuseSignIn], syncs with [syncStatus], and takes a new
/// item, answering with it as the server keeps it. [asked] has each call.
class StandIn {
  StandIn._(this._server);

  final HttpServer _server;
  final asked = <String>[];
  bool refuseSignIn = false;
  int syncStatus = 200;
  final Map<String, dynamic> sync =
      jsonDecode(File('test/sample/sync.json').readAsStringSync())
          as Map<String, dynamic>;

  String get url => 'http://127.0.0.1:${_server.port}';

  /// Started, and answering, outside the test's fake clock.
  static Future<StandIn> start(WidgetTester tester) async {
    final standIn = (await tester.runAsync(() async {
      final made = StandIn._(
        await HttpServer.bind(InternetAddress.loopbackIPv4, 0),
      );
      made._server.listen(made._answer);
      return made;
    }))!;
    addTearDown(() => tester.runAsync(standIn.stop));
    return standIn;
  }

  /// Nothing answers there any more: this device can't reach it.
  Future<void> stop() => _server.close(force: true);

  Future<void> _answer(HttpRequest request) async {
    final path = request.uri.path;
    asked.add('${request.method} $path');
    final body = await utf8.decoder.bind(request).join();
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
      case '/identity/connect/token' when refuseSignIn:
        // As the server answers a wrong password and an email with no
        // account alike.
        json(400, {
          'error': 'invalid_grant',
          'error_description': 'Username or password is incorrect. Try again',
          'ErrorModel': {
            'Message': 'Username or password is incorrect. Try again',
            'Object': 'error',
          },
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
        json(syncStatus, syncStatus == 200 ? sync : null);
      case '/api/ciphers':
        final saved = jsonDecode(body) as Map<String, dynamic>;
        json(200, {
          ...saved,
          'id': '0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d',
          'object': 'cipherDetails',
          'revisionDate': '2026-11-07T10:00:00.000Z',
          'edit': true,
          'viewPassword': true,
        });
      default:
        json(404, null);
    }
    await request.response.close();
  }
}

void main() {
  setUpAll(loadFonts);

  setUp(() {
    // The sign-in screen on a computer, with nothing kept.
    openEmpty();
  });

  Future<void> signInScreen(WidgetTester tester, String server) async {
    tester.view.physicalSize = const Size(1280, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(const PassBundleApp());
    await tester.tap(find.text('Self-hosted?'));
    await tester.pump();
    await tester.enterText(find.widgetWithText(TextField, 'Server'), server);
  }

  Future<void> signIn(WidgetTester tester, String email, String password) async {
    await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
    await tester.enterText(
      find.widgetWithText(TextField, 'Master password'),
      password,
    );
    await tester.tap(find.text('Sign in'));
  }

  Finder underServer(Finder text) =>
      find.descendant(of: find.byKey(const ValueKey('server')), matching: text);

  testWidgets(
    'an address that isn\'t a PassBundle server is said under the server field, as soon as the person leaves it',
    (tester) async {
      // A web server that answers every address with a web page.
      final server = (await tester.runAsync(() async {
        final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
        server.listen((request) async {
          await utf8.decoder.bind(request).join();
          request.response.headers.contentType = ContentType.html;
          request.response.write(
            '<!DOCTYPE html><html><body>Welcome to nginx!</body></html>',
          );
          await request.response.close();
        });
        return server;
      }))!;
      addTearDown(() => tester.runAsync(() => server.close(force: true)));
      final host = '127.0.0.1:${server.port}';
      await signInScreen(tester, 'http://$host');
      // Leaving the field checks it.
      await tester.tap(find.widgetWithText(TextField, 'Email'));
      final notAServer = find.text(
        'There\'s no PassBundle server at $host. Check the address: it\'s where you open your web vault.',
      );
      await pumpUntil(tester, underServer(notAServer));

      // Signing in says the same, there, and nothing under the password.
      await signIn(tester, 'ana@example.test', 'anything');
      await pumpUntil(tester, underServer(notAServer));
      expect(notAServer, findsOneWidget);
      // The page itself is never quoted.
      expect(find.textContaining('nginx'), findsNothing);
      expect(find.textContaining('DOCTYPE'), findsNothing);

      // Typing again takes the words away until the next check.
      await tester.enterText(
        find.widgetWithText(TextField, 'Server'),
        'http://$host/',
      );
      await tester.pump();
      expect(notAServer, findsNothing);
    },
  );

  testWidgets(
    'a wrong email or master password is said the same, never which one',
    (tester) async {
      final standIn = await StandIn.start(tester);
      standIn.refuseSignIn = true;
      await signInScreen(tester, standIn.url);
      const wrong = 'That email or master password isn\'t right.';
      // An email with no account.
      await signIn(tester, 'nobody@example.test', samplePassword);
      await pumpUntil(tester, find.text(wrong));
      expect(underServer(find.text(wrong)), findsNothing);
      // The right email, a wrong password: the server takes it, but the
      // account's key doesn't open, and it's said the same.
      standIn.refuseSignIn = false;
      await signIn(tester, 'ana.lopez@example.test', 'not the password');
      await pumpUntil(tester, find.text(wrong));
      expect(find.text('That master password isn\'t right.'), findsNothing);
      expect(
        standIn.asked.where((a) => a == 'POST /identity/connect/token'),
        hasLength(2),
      );
    },
  );

  testWidgets(
    'offline, the vault says it shows this device\'s copy, and saving says it didn\'t, keeping what was typed',
    (tester) async {
      final standIn = await StandIn.start(tester);
      await signInScreen(tester, standIn.url);
      await signIn(tester, 'ana.lopez@example.test', samplePassword);
      await pumpUntil(tester, find.text('Example Mail'));
      // The sync as it opens, done.
      await pumpUntil(tester, find.byTooltip('Sync'));
      expect(find.byKey(const ValueKey('offline')), findsNothing);

      // The server goes away: the next sync says what's shown, once, with
      // no error of the network's.
      await tester.runAsync(standIn.stop);
      await tester.tap(find.byTooltip('Sync'));
      await pumpUntil(tester, find.byKey(const ValueKey('offline')));
      expect(
        find.textContaining('You\'re seeing the copy on this device'),
        findsOneWidget,
      );
      expect(find.textContaining('os error'), findsNothing);

      // Saving needs the server: the editor stays, with what was typed.
      await tester.tap(find.text('New'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Login'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.widgetWithText(TextField, 'Name'),
        'Typed offline',
      );
      await tester.tap(find.text('Save'));
      await pumpUntil(tester, find.textContaining('Not saved.'));
      expect(
        find.textContaining('What you typed is still here.'),
        findsOneWidget,
      );
      expect(find.widgetWithText(TextField, 'Typed offline'), findsOneWidget);
    },
  );

  testWidgets(
    'a new item the server took is saved once, though the sync after it fails',
    (tester) async {
      final standIn = await StandIn.start(tester);
      await signInScreen(tester, standIn.url);
      await signIn(tester, 'ana.lopez@example.test', samplePassword);
      await pumpUntil(tester, find.text('Example Mail'));
      // The sync as it opens, done: the next one fails.
      await pumpUntil(tester, find.byTooltip('Sync'));
      standIn.syncStatus = 503;
      await tester.tap(find.text('New'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Login'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.widgetWithText(TextField, 'Name'),
        'Saved once',
      );
      await tester.tap(find.text('Save'));
      // The editor closes, saying so: there's no Save to press again.
      await pumpUntil(
        tester,
        find.text(
          'Saved. PassBundle couldn\'t sync the rest just now, and will next time.',
        ),
      );
      // Gone, once its way out is done.
      await tester.pump(const Duration(milliseconds: 600));
      expect(find.widgetWithText(TextField, 'Name'), findsNothing);
      // It shows already, and the server was asked to save it once.
      expect(find.text('Saved once'), findsOneWidget);
      expect(
        standIn.asked.where((a) => a == 'POST /api/ciphers'),
        hasLength(1),
      );
    },
  );
}
