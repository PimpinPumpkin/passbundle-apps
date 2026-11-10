// The apps' rough edges, smoothed (the gap audit's C4, C5, C8, C10, C11,
// C12), against the real core: biometric unlock offered only where it can
// work, signing out from a computer's Settings by name, Back going back a
// step, what opens in the web vault saying so, empty lists saying why, and
// a phone's lock screen asking for the biometric by itself and offering the
// master password hint.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/vault.dart';

import 'support.dart';

void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// What a PassBundle server answers about itself, without signing in
/// (`GET /api/config`): the sign-in screen checks a self-hosted server with
/// it as the person leaves the Server field (docs/core.md, "Checking a
/// server").
const passBundleConfig = (
  200,
  {
    'version': '2026.6.0',
    'server': {'name': 'PassBundle'},
  },
);

/// A server on this computer that answers each path with [answers]' JSON
/// (status, body), its configuration as a PassBundle server does unless
/// [answers] says otherwise, and 404 otherwise; started outside the test's
/// fake clock. Each request's method and path go in [asked].
Future<String> answering(
  WidgetTester tester,
  Map<String, (int, Object)> answers, {
  List<String>? asked,
}) async {
  final answered = {'/api/config': passBundleConfig, ...answers};
  final server = (await tester.runAsync(() async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.listen((request) async {
      asked?.add('${request.method} ${request.uri.path}');
      await utf8.decoder.bind(request).join();
      final (status, body) = answered[request.uri.path] ?? (404, const {});
      request.response.statusCode = status;
      request.response.headers.contentType = ContentType.json;
      request.response.write(jsonEncode(body));
      await request.response.close();
    });
    return server;
  }))!;
  addTearDown(() => tester.runAsync(() => server.close(force: true)));
  return 'http://127.0.0.1:${server.port}';
}

/// The sample vault kept here, locked, on [server] instead of its own.
void openSampleOn(String server) {
  final dir = Directory.systemTemp.createTempSync('pb-app-test-').path;
  final settings =
      jsonDecode(File('test/sample/settings.json').readAsStringSync())
          as Map<String, dynamic>;
  File(
    '$dir/settings.json',
  ).writeAsStringSync(jsonEncode({...settings, 'server': server}));
  File('test/sample/sync.json').copySync('$dir/sync.json');
  Core().openAt(dir);
  machineDoesntLock(false);
}

Future<void> unlockOnComputer(WidgetTester tester) async {
  // Tall enough for the whole menu, Send and Import from browsers too.
  setSize(tester, const Size(1280, 980));
  openSample();
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text('Example Mail'));
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
}

Future<void> openSettings(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Settings'));
  await tester.pump();
  await pumpUntil(tester, find.text('Lock after'));
  await pumpUntilGone(tester, find.byKey(const ValueKey('unlock-loading')));
  await tester.pump(const Duration(milliseconds: 400));
}

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'a computer offers unlocking with biometrics only where it can work, and says how approvals confirm there',
    (tester) async {
      await unlockOnComputer(tester);
      await openSettings(tester);
      if (Platform.isWindows) {
        // Windows Hello, through the desktop agent (not running here).
        expect(find.text('Unlock with Windows Hello'), findsOneWidget);
        expect(
          find.text('With Windows Hello or your master password'),
          findsOneWidget,
        );
      } else {
        // Linux has no agent. The Mac app's Touch ID is its own, and has
        // tests of its own (mac_touch_id_test.dart): without its stand-in,
        // a Mac here goes the computer's way, as Linux does.
        expect(find.textContaining('Unlock with'), findsNothing);
        expect(find.textContaining('Touch ID'), findsNothing);
        expect(find.text('Unlock'), findsNothing);
        expect(find.text('With your master password'), findsOneWidget);
      }
    },
  );

  testWidgets(
    'a computer\'s Settings signs out of the account shown, by name',
    (tester) async {
      await unlockOnComputer(tester);
      await openSettings(tester);
      final signOut = find.byKey(const ValueKey('settings-sign-out'));
      await tester.ensureVisible(signOut);
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: signOut,
          matching: find.text('Sign out of ana.lopez@example.test'),
        ),
        findsOneWidget,
      );
      await tester.tap(signOut);
      await pumpUntil(tester, find.text('Sign out?'));
      await tester.pumpAndSettle();
      // Settings closed first; the question names the account.
      expect(find.text('Lock after'), findsNothing);
      expect(
        find.textContaining('the vault of ana.lopez@example.test'),
        findsOneWidget,
      );
      await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
      await pumpUntil(tester, find.text('Sign in to PassBundle'));
    },
  );

  testWidgets(
    'Back goes back a step: from making an account, and from two-step login, to signing in',
    (tester) async {
      setSize(tester, const Size(390, 844));
      // A PassBundle server that asks for an authenticator app's code.
      final asked = <String>[];
      final server = await answering(tester, asked: asked, {
        '/identity/accounts/prelogin': (
          200,
          {
            'kdf': 0,
            'kdfIterations': 600000,
            'kdfMemory': null,
            'kdfParallelism': null,
          },
        ),
        '/identity/connect/token': (
          400,
          {
            'error': 'invalid_grant',
            'error_description': 'Two factor required.',
            'TwoFactorProviders': ['0'],
            'TwoFactorProviders2': {'0': null},
          },
        ),
      });
      openEmpty();
      await tester.pumpWidget(const PassBundleApp());
      await tester.tap(find.text('Create account'));
      await tester.pump();
      expect(find.text('Create a PassBundle account'), findsOneWidget);
      await tester.binding.handlePopRoute();
      await tester.pump();
      expect(find.text('Sign in to PassBundle'), findsOneWidget);

      await tester.tap(find.text('Self-hosted?'));
      await tester.pump();
      await tester.enterText(find.widgetWithText(TextField, 'Server'), server);
      await tester.enterText(
        find.widgetWithText(TextField, 'Email'),
        'ana@example.test',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        'anything at all',
      );
      await tester.tap(find.text('Sign in'));
      await pumpUntil(tester, find.text('Two-step login'));
      await tester.binding.handlePopRoute();
      await tester.pump();
      expect(find.text('Two-step login'), findsNothing);
      expect(find.text('Sign in to PassBundle'), findsOneWidget);

      // Leaving the Server field checked it, and the check's answer is in:
      // a PassBundle server, so nothing is said under the field.
      for (var i = 0; i < 100 && !asked.contains('GET /api/config'); i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)),
        );
      }
      expect(asked, contains('GET /api/config'));
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 500)),
      );
      await tester.pump();
      final field = tester.widget<TextField>(
        find.byKey(const ValueKey('server')),
      );
      expect(field.decoration?.errorText, isNull);
    },
  );

  testWidgets('Back from adding an account goes back to the one shown', (
    tester,
  ) async {
    setSize(tester, const Size(1280, 900));
    openAccounts();
    await tester.pumpWidget(const PassBundleApp());
    await tester.enterText(find.byType(TextField), samplePassword);
    await tester.tap(find.text('Unlock'));
    await pumpUntil(tester, find.text('Example Mail'));
    await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
    await tester.tap(find.byTooltip('Accounts'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add account'));
    await tester.pumpAndSettle();
    expect(find.text('Add an account'), findsOneWidget);
    await tester.binding.handlePopRoute();
    await pumpUntil(tester, find.text('Example Mail'));
    expect(find.text('Add an account'), findsNothing);
  });

  testWidgets(
    'what the app doesn\'t edit says where it\'s edited, the web vault\'s pages say they open there, and empty lists say why',
    (tester) async {
      await unlockOnComputer(tester);
      final menu = find.byType(Sidebar);
      // Import and Export, each the web vault's.
      for (final tool in ['Import', 'Export']) {
        expect(
          find.descendant(of: menu, matching: find.text(tool)),
          findsOneWidget,
        );
      }
      expect(find.text('Import and export'), findsNothing);
      expect(find.byTooltip('Opens the web vault'), findsNWidgets(2));

      // An SSH key: the web vault edits it, and the item says so.
      await tester.tap(find.text('Build server'));
      await tester.pump();
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsNothing);
      expect(find.byKey(const ValueKey('edit-in-web-vault')), findsOneWidget);
      expect(find.text('Edit in web vault'), findsOneWidget);
      // A login: the app edits it.
      await tester.tap(find.text('Example Mail'));
      await tester.pump();
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsOneWidget);
      expect(find.byKey(const ValueKey('edit-in-web-vault')), findsNothing);

      // A search that finds nothing says what it looked for.
      await tester.enterText(
        find.widgetWithText(TextField, 'Search items'),
        'zzz',
      );
      await tester.pump();
      expect(find.text('Nothing matches “zzz”.'), findsOneWidget);
    },
  );

  testWidgets(
    'a phone\'s Settings opens import and export in the web vault',
    (tester) async {
      setSize(tester, const Size(390, 844));
      PhoneStandIn(tester, status: 'unavailable');
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Logins'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      await tester.tap(find.byTooltip('Settings'));
      await tester.pumpAndSettle();
      for (final row in ['web-import', 'web-export', 'web-vault']) {
        await tester.ensureVisible(find.byKey(ValueKey(row)));
        await tester.pumpAndSettle();
        expect(find.byKey(ValueKey(row)), findsOneWidget);
      }
      expect(find.text('Web vault'), findsOneWidget);
    },
  );

  testWidgets(
    'a phone\'s lock screen asks for the biometric by itself as the app opens and comes back, not after Lock',
    (tester) async {
      setSize(tester, const Size(390, 844));
      final phone = PhoneStandIn(tester, status: 'setUp');
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      Future<void> promptedTimes(int times) async {
        for (var i = 0; i < 100 && phone.unlockPrompts < times; i++) {
          await tester.pump(const Duration(milliseconds: 50));
        }
        await tester.pump(const Duration(milliseconds: 200));
        expect(phone.unlockPrompts, times);
      }

      // As it opens: the prompt, once (the person cancels it here).
      await pumpUntil(tester, find.text('Unlock with fingerprint'));
      await promptedTimes(1);

      // Unlocked with the master password, then locked by the person: the
      // lock screen waits for them.
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Logins'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Lock'));
      await pumpUntil(tester, find.text('Unlock with fingerprint'));
      await promptedTimes(1);

      // Away and back, locked: it asks again.
      final binding = tester.binding;
      binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
      binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await promptedTimes(2);
    },
  );

  testWidgets(
    'the lock screen gets the master password hint from the account\'s server',
    (tester) async {
      setSize(tester, const Size(1280, 900));
      final server = await answering(tester, {
        // A server without email, set to show hints.
        '/api/accounts/password-hint': (
          400,
          {
            'message': 'Your password hint is: the blue door',
            'object': 'error',
          },
        ),
      });
      openSampleOn(server);
      await tester.pumpWidget(const PassBundleApp());
      await pumpUntil(tester, find.text('Unlock PassBundle'));
      await tester.tap(find.text('Get your master password hint'));
      await pumpUntil(tester, find.text('Your hint: the blue door'));
      await tester.tap(find.text('OK'));
      await tester.pumpAndSettle();
      expect(find.text('Unlock PassBundle'), findsOneWidget);
    },
  );
}
