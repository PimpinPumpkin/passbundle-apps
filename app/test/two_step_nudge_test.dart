// The nudges about two-step login above the vault's list (two_step_nudge.dart;
// DESIGN.md in the meta repo, "A nudge for everyone else"), through the real
// core to a stand-in for PassBundle's server on this computer: the server's
// answer at each sync decides which shows, in the web vault's words; Set it
// up opens the web vault's two-step settings in the browser, and coming back
// to the app syncs; Not now tells the server and hides it; an older server's
// `show` is the first nudge, and a server without it, or one that can't be
// reached, shows nothing. On a phone it's above the list on the Items tab
// only. With two accounts, each shows its own server's answer, and "All
// accounts" shows none. The core's own tests (core/src/server.rs,
// core/tests/server.rs) and the meta repo's end-to-end tests have the
// server's side.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/two_step_nudge.dart';
import 'package:passbundle/vault.dart';

import 'support.dart';

const setUpWords =
    'Your password is all that protects your account. Add a second step, such as an authenticator app, so a stolen password isn\'t enough.';
const strongerWords =
    'Codes by email are only as safe as your email account. An authenticator app or a security key is safer.';

const nudgePath = '/api/keykeeper/two-step-nudge';

/// A stand-in for PassBundle's server: the sample vault's account, and the
/// nudge it says ([nudge], as the server's answer; null for a server without
/// it). Not now makes it say none from then on, as the server keeps it.
/// [asked] has each call. Given a [userId], its account is another person's
/// than the sample's (the same email and password), as a second account on
/// another server is: the app refuses one it has already.
class StandIn {
  StandIn._(this._server, this.nudge);

  final HttpServer _server;
  final asked = <String>[];
  Object? nudge;
  final Map<String, dynamic> sync =
      jsonDecode(File('test/sample/sync.json').readAsStringSync())
          as Map<String, dynamic>;
  bool _stopped = false;

  String get url => 'http://127.0.0.1:${_server.port}';

  int get nudgeAsked => asked.where((a) => a == 'GET $nudgePath').length;

  static Future<StandIn> start(
    WidgetTester tester,
    Object? nudge, {
    String? userId,
  }) async {
    final standIn = (await tester.runAsync(() async {
      final made = StandIn._(
        await HttpServer.bind(InternetAddress.loopbackIPv4, 0),
        nudge,
      );
      if (userId != null) (made.sync['profile'] as Map)['id'] = userId;
      made._server.listen(made._answer);
      return made;
    }))!;
    addTearDown(() => tester.runAsync(standIn.stop));
    return standIn;
  }

  /// Gone, as a server that can't be reached.
  Future<void> stop() async {
    if (_stopped) return;
    _stopped = true;
    await _server.close(force: true);
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
      case '/api/keykeeper/two-step-nudge' when request.method == 'GET':
        json(nudge == null ? 404 : 200, nudge);
      case '/api/keykeeper/two-step-nudge/dismiss'
          when request.method == 'POST':
        nudge = {'show': false, 'nudge': null};
        json(200, nudge);
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

/// The browser, stood in for: the addresses it was asked to open.
List<String> browser() {
  final opened = <String>[];
  Core.debugOpenWebsite = (url) async {
    opened.add(url);
    return true;
  };
  addTearDown(() => Core.debugOpenWebsite = null);
  return opened;
}

Finder get nudgeBanner => find.byType(TwoStepNudgeBanner);

void main() {
  setUpAll(loadFonts);

  const computer = Size(1280, 1100);
  const phone = Size(390, 844);

  /// Waits until [standIn] has been asked about the nudge [times] times in
  /// all, and the sync that asked is in (the screens show Sync again).
  Future<void> askedTimes(
    WidgetTester tester,
    StandIn standIn,
    int times,
  ) async {
    final end = DateTime.now().add(const Duration(seconds: 20));
    while (standIn.nudgeAsked < times) {
      if (DateTime.now().isAfter(end)) {
        throw TestFailure(
          'Asked about the nudge ${standIn.nudgeAsked} times, not $times',
        );
      }
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 50)),
      );
      await tester.pump();
    }
    await pumpUntil(tester, find.byTooltip('Sync'));
    await tester.pump();
  }

  /// Waits until the stand-in heard Not now.
  Future<void> dismissed(WidgetTester tester, StandIn standIn) async {
    final end = DateTime.now().add(const Duration(seconds: 20));
    while (!standIn.asked.contains('POST $nudgePath/dismiss')) {
      if (DateTime.now().isAfter(end)) throw TestFailure('Never told Not now');
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 50)),
      );
      await tester.pump();
    }
  }

  /// Signs in to [standIn] on the sign-in screen showing, as the sample
  /// vault's account, until the vault's first sync is in.
  Future<void> signInTo(WidgetTester tester, StandIn standIn) async {
    final before = standIn.nudgeAsked;
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
    await askedTimes(tester, standIn, before + 1);
  }

  /// Signed in to a stand-in that says [nudge], at [size].
  Future<StandIn> signIn(
    WidgetTester tester,
    Object? nudge, {
    Size size = computer,
  }) async {
    final standIn = await StandIn.start(tester, nudge);
    openEmpty();
    setSize(tester, size);
    await tester.pumpWidget(const PassBundleApp());
    await signInTo(tester, standIn);
    return standIn;
  }

  /// A phone's tab, from the bar at its foot.
  Future<void> tab(WidgetTester tester, String name) async {
    await tester.tap(
      find.descendant(of: find.byType(NavigationBar), matching: find.text(name)),
    );
    await tester.pumpAndSettle();
  }

  testWidgets(
    'on a computer, the nudge to set up two-step login is above the list; Set it up opens the web vault\'s two-step settings, and coming back syncs',
    (tester) async {
      final opened = browser();
      final standIn = await signIn(tester, {'show': true, 'nudge': 'setUp'});
      expect(find.text(setUpWords), findsOneWidget);
      expect(find.text(strongerWords), findsNothing);
      // A callout in the list's column, under its search and above its
      // rows, never a dialog.
      expect(
        find.descendant(of: find.byType(ItemList), matching: nudgeBanner),
        findsOneWidget,
      );
      expect(find.byType(Dialog), findsNothing);
      expect(
        tester.getBottomLeft(nudgeBanner).dy,
        lessThanOrEqualTo(
          tester.getTopLeft(find.widgetWithText(ItemRow, 'Example Mail')).dy,
        ),
      );

      await tester.tap(find.text('Set it up'));
      await tester.pump();
      expect(opened, ['${standIn.url}$webVaultTwoStepPath']);
      expect(opened.single, endsWith('/#/settings/security/two-factor'));
      // Set up in the browser meanwhile: back in the app, it syncs, and the
      // server says none.
      standIn.nudge = {'show': false, 'nudge': null};
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await askedTimes(tester, standIn, 2);
      expect(nudgeBanner, findsNothing);
      // Coming back again, without Set it up, doesn't sync.
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 500)),
      );
      await tester.pump();
      expect(standIn.nudgeAsked, 2);
    },
  );

  testWidgets(
    'Not now tells the server and hides it, and the next sync leaves it hidden',
    (tester) async {
      final standIn = await signIn(tester, {'show': true, 'nudge': 'setUp'});
      expect(find.text(setUpWords), findsOneWidget);
      await tester.tap(find.text('Not now'));
      await tester.pump();
      expect(nudgeBanner, findsNothing);
      await dismissed(tester, standIn);

      await tester.tap(find.byTooltip('Sync'));
      await askedTimes(tester, standIn, 2);
      expect(nudgeBanner, findsNothing);
      // The list is as it was.
      expect(find.widgetWithText(ItemRow, 'Example Mail'), findsOneWidget);
    },
  );

  testWidgets(
    'someone whose only second step is email codes is asked for a stronger method, with the same Set it up and Not now',
    (tester) async {
      final opened = browser();
      final standIn = await signIn(tester, {
        'show': false,
        'nudge': 'stronger',
      });
      expect(find.text(strongerWords), findsOneWidget);
      expect(find.text(setUpWords), findsNothing);
      await tester.tap(find.text('Set it up'));
      await tester.pump();
      expect(opened, ['${standIn.url}$webVaultTwoStepPath']);
      await tester.tap(find.text('Not now'));
      await tester.pump();
      expect(nudgeBanner, findsNothing);
      await dismissed(tester, standIn);
    },
  );

  testWidgets(
    'an older server\'s show is the first nudge, and once the server can\'t be reached it goes',
    (tester) async {
      final standIn = await signIn(tester, {'show': true});
      expect(find.text(setUpWords), findsOneWidget);

      // Unreachable: the vault says it's offline, and the nudge goes.
      await tester.runAsync(standIn.stop);
      await tester.tap(find.byTooltip('Sync'));
      await pumpUntil(tester, find.textContaining('Can\'t reach your server.'));
      expect(nudgeBanner, findsNothing);
    },
  );

  testWidgets(
    'a server without the nudge, or one naming a nudge the app doesn\'t know, shows nothing',
    (tester) async {
      final standIn = await signIn(tester, null);
      expect(standIn.asked, contains('GET /api/sync'));
      expect(nudgeBanner, findsNothing);
      expect(find.textContaining('Can\'t reach your server.'), findsNothing);

      standIn.nudge = {'show': false, 'nudge': 'somethingNew'};
      await tester.tap(find.byTooltip('Sync'));
      await askedTimes(tester, standIn, 2);
      expect(nudgeBanner, findsNothing);
    },
  );

  testWidgets(
    'on a phone, the nudge is above the list on the Items tab only, and Not now hides it',
    (tester) async {
      PhoneStandIn(tester, status: 'unavailable');
      final standIn = await signIn(tester, {
        'show': true,
        'nudge': 'setUp',
      }, size: phone);
      await pumpUntil(tester, find.text(setUpWords));
      // Under the search, above the list.
      expect(
        tester.getTopLeft(nudgeBanner).dy,
        greaterThan(tester.getTopLeft(find.byType(TextField).first).dy),
      );
      expect(
        tester.getBottomLeft(nudgeBanner).dy,
        lessThanOrEqualTo(
          tester.getTopLeft(find.byKey(const ValueKey('vault-scroll'))).dy,
        ),
      );
      // Not on Settings. (The Items tab stays built behind it, unseen.)
      await tab(tester, 'Settings');
      expect(nudgeBanner.hitTestable(), findsNothing);
      await tab(tester, 'Items');
      expect(nudgeBanner.hitTestable(), findsOneWidget);

      await tester.tap(find.text('Not now'));
      await tester.pump();
      expect(nudgeBanner, findsNothing);
      await dismissed(tester, standIn);
    },
  );

  testWidgets(
    'with two accounts, each shows its own server\'s answer, and All accounts shows none',
    (tester) async {
      final work = await signIn(tester, {'show': true, 'nudge': 'setUp'});
      expect(find.text(setUpWords), findsOneWidget);
      final first = Core.account;

      // A second account, on a server that says none.
      final home = await StandIn.start(
        tester,
        {'show': false, 'nudge': null},
        userId: '0d4c7b1e-3f2a-4e6b-8c9d-5a1b2c3d4e5f',
      );
      await tester.tap(find.byTooltip('Accounts'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('add-account')));
      await tester.pumpAndSettle();
      expect(find.text('Add an account'), findsOneWidget);
      await signInTo(tester, home);
      expect(Core.account, isNot(first));
      expect(nudgeBanner, findsNothing);

      // Back to the first: its own server's answer, at its own sync.
      await tester.tap(find.byTooltip('Accounts'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey('account-$first')));
      // Not pumpAndSettle: its vault syncs on opening, on an isolate of its
      // own, with a spinner going until it's in.
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 500));
      await askedTimes(tester, work, 2);
      expect(find.text(setUpWords), findsOneWidget);
      expect(home.nudgeAsked, 1);

      // All accounts lists both accounts' items: no one account's nudge.
      await tester.tap(await menuEntry(tester, 'All accounts'));
      await tester.pumpAndSettle();
      expect(nudgeBanner, findsNothing);
      await tester.tap(await menuEntry(tester, 'All items'));
      await tester.pumpAndSettle();
      expect(nudgeBanner, findsOneWidget);
    },
  );
}
