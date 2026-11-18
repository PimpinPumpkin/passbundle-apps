// Signing in with a device (device_sign_in.dart; DESIGN.md in the meta repo,
// "Our apps asking"), with a stand-in for the core's calls: "Sign in with a
// device" on the sign-in screen asks with the email typed; the fingerprint
// phrase shows first and largest; the core is asked every 5 seconds until
// the sign-in is done or two-step login follows; Resend sends a new request
// with a new phrase; a denial, expiry, a key that doesn't work, and an
// approval the server won't take say so, with a new request or the master
// password; a refusal of the sign-in (an organization's rules) goes back to
// the master password with its words, as a password sign-in shows them; a
// moment offline keeps trying; and "Use master password instead" gives the
// request up. Desktop and phone. The core's side (the key pair, the code,
// opening the key, the sign-in) has its own tests, against a real server.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/device_sign_in.dart';
import 'package:passbundle/sign_in.dart';
import 'package:passbundle/theme.dart';

import 'support.dart';

const phrase = 'pacifier-unvisited-amuser-sublime-tropics';
const newPhrase = 'annex-mortally-radial-deplored-cheer';
const email = 'dana@example.test';

/// A request as the core answers with it (core/src/ffi.rs, `device_sign_in`).
Map<String, dynamic> asked({
  String status = 'waiting',
  String fingerprint = phrase,
  int left = 900,
}) => {
  'ok': true,
  'step': 'deviceSignIn',
  'request': {
    'email': email,
    'fingerprint': fingerprint,
    'status': status,
    'secondsLeft': status == 'waiting' ? left : 0,
  },
};

const done = {'ok': true, 'step': 'done'};

/// The core's calls, answered in order (the last again once they run out),
/// and what was asked.
class FakeCalls extends DeviceSignInCalls {
  FakeCalls({this.starts = const [], this.statuses = const []});

  final List<Map<String, dynamic>> starts;
  final List<Map<String, dynamic>> statuses;
  final asked = <String>[];

  Answer _next(List<Map<String, dynamic>> answers, String kind) {
    final at = asked.where((a) => a.startsWith(kind)).length - 1;
    return Answer(at < answers.length ? answers[at] : answers.last);
  }

  @override
  Future<Answer> start(String server, String email) async {
    asked.add('start $server $email');
    return _next(starts, 'start');
  }

  @override
  Future<Answer> status() async {
    asked.add('status');
    return _next(statuses, 'status');
  }

  @override
  Future<void> giveUp() async => asked.add('giveUp');
}

/// A text's font size, as drawn: its own style over the one around it.
double? sizeOf(WidgetTester tester, Finder text) => DefaultTextStyle.of(
  tester.element(text),
).style.merge(tester.widget<Text>(text).style).fontSize;

void main() {
  setUpAll(loadFonts);

  /// A screen size: a computer's window, or a phone's (with its own look).
  void sized(WidgetTester tester, {bool phone = false}) {
    tester.view.physicalSize = phone
        ? const Size(390, 844)
        : const Size(1280, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    if (phone) {
      debugPhone = true;
      addTearDown(() => debugPhone = null);
    }
  }

  /// The request's screen on its own, as the sign-in screen shows it.
  Future<({FakeCalls calls, List<String> heard})> show(
    WidgetTester tester,
    Map<String, dynamic> answer, {
    List<Map<String, dynamic>> starts = const [],
    List<Map<String, dynamic>> statuses = const [],
    bool phone = false,
  }) async {
    sized(tester, phone: phone);
    final calls = FakeCalls(
      starts: starts.isEmpty ? [answer] : starts,
      statuses: statuses.isEmpty ? [answer] : statuses,
    );
    final heard = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        theme: passBundleTheme(Brightness.light),
        home: DeviceSignInScreen(
          key: UniqueKey(),
          server: 'https://vault.example.test',
          answer: answer,
          calls: calls,
          onSignedIn: () => heard.add('signedIn'),
          onTwoStep: (_) => heard.add('twoStep'),
          onPassword: (said) => heard.add('password ${said ?? ''}'.trim()),
        ),
      ),
    );
    return (calls: calls, heard: heard);
  }

  /// The sign-in screen, with the core on an empty folder and a stand-in
  /// for its calls to sign in with a device.
  Future<FakeCalls> signInScreen(
    WidgetTester tester, {
    List<Map<String, dynamic>> starts = const [],
    List<Map<String, dynamic>> statuses = const [],
  }) async {
    openEmpty();
    sized(tester);
    final calls = FakeCalls(
      starts: starts.isEmpty ? [asked()] : starts,
      statuses: statuses.isEmpty ? [asked()] : statuses,
    );
    debugDeviceSignIn = calls;
    addTearDown(() => debugDeviceSignIn = null);
    await tester.pumpWidget(
      MaterialApp(
        theme: passBundleTheme(Brightness.light),
        home: SignInScreen(onSignedIn: () {}),
      ),
    );
    return calls;
  }

  final withDevice = find.byKey(const ValueKey('sign-in-with-device'));
  final shownPhrase = find.byKey(const ValueKey('device-sign-in-phrase'));
  final resend = find.byKey(const ValueKey('device-sign-in-resend'));
  final usePassword = find.byKey(const ValueKey('device-sign-in-password'));

  testWidgets(
    '"Sign in with a device" asks with the email typed, and the phrase shows first and largest',
    (tester) async {
      final calls = await signInScreen(tester);
      expect(withDevice, findsOneWidget);
      // It's beside the master password: under Sign in, before the hint.
      expect(
        tester.getRect(withDevice).top,
        greaterThan(tester.getRect(find.text('Sign in')).bottom),
      );
      expect(
        tester.getRect(withDevice).bottom,
        lessThanOrEqualTo(
          tester.getRect(find.text('Get your master password hint')).top,
        ),
      );

      // No email yet: said under it, and nothing asked.
      await tester.tap(withDevice);
      await tester.pump();
      expect(
        find.text('Type your email first, then sign in with a device.'),
        findsOneWidget,
      );
      expect(calls.asked, isEmpty);

      await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        'typed by mistake',
      );
      await tester.tap(withDevice);
      await tester.pump();
      await tester.pump();
      expect(calls.asked.single, startsWith('start https://'));
      expect(calls.asked.single, endsWith(' $email'));

      expect(find.text('Sign in with a device'), findsOneWidget);
      expect(find.text(email), findsOneWidget);
      expect(
        find.descendant(of: shownPhrase, matching: find.text(phrase)),
        findsOneWidget,
      );
      // First: above what to do, and the buttons.
      final top = tester.getRect(shownPhrase).top;
      final explained = find.textContaining(
        'Approve it only if that device shows this same phrase.',
      );
      expect(top, lessThan(tester.getRect(explained).top));
      expect(top, lessThan(tester.getRect(resend).top));
      // Largest: bigger than the screen's title and everything under it.
      final phraseSize = sizeOf(tester, find.text(phrase))!;
      for (final other in [
        find.text('Sign in with a device'),
        find.text(email),
        explained,
        find.text('Fingerprint phrase'),
        find.byKey(const ValueKey('device-sign-in-waiting')),
      ]) {
        expect(phraseSize, greaterThan(sizeOf(tester, other)!));
      }
      expect(
        find.text('Waiting for approval. Expires in 15 minutes.'),
        findsOneWidget,
      );
      expect(find.text('Resend'), findsOneWidget);
      expect(find.text('Use master password instead'), findsOneWidget);
    },
  );

  testWidgets('it asks every 5 seconds, and signs in once approved', (
    tester,
  ) async {
    final shown = await show(tester, asked(), statuses: [asked(), done]);
    // Nothing is asked before the first 5 seconds are up.
    await tester.pump(const Duration(seconds: 4));
    expect(shown.calls.asked, isEmpty);
    await tester.pump(const Duration(seconds: 1));
    expect(shown.calls.asked, ['status']);
    expect(shown.heard, isEmpty);
    await tester.pump(const Duration(seconds: 5));
    expect(shown.calls.asked, ['status', 'status']);
    expect(shown.heard, ['signedIn']);
    // Signed in: it stops asking.
    await tester.pump(const Duration(seconds: 15));
    expect(shown.calls.asked.length, 2);
  });

  testWidgets('Resend sends a new request, with a new phrase', (tester) async {
    final shown = await show(
      tester,
      asked(),
      starts: [asked(fingerprint: newPhrase)],
    );
    expect(find.text(phrase), findsOneWidget);
    await tester.tap(resend);
    await tester.pump();
    expect(shown.calls.asked, ['start https://vault.example.test $email']);
    expect(find.text(phrase), findsNothing);
    expect(
      find.descendant(of: shownPhrase, matching: find.text(newPhrase)),
      findsOneWidget,
    );
    // And it goes on asking about the new one.
    await tester.pump(const Duration(seconds: 5));
    expect(shown.calls.asked.last, 'status');
  });

  testWidgets(
    'a denial says so and stops, with a new request or the master password',
    (tester) async {
      final shown = await show(
        tester,
        asked(),
        statuses: [asked(status: 'denied')],
      );
      await tester.pump(const Duration(seconds: 5));
      expect(find.text('Sign-in denied'), findsOneWidget);
      expect(
        find.text(
          'This sign-in was denied on another of your devices. If this was you, try to sign in with the device again.',
        ),
        findsOneWidget,
      );
      expect(find.text(phrase), findsNothing);
      // Denied: it stops asking.
      await tester.pump(const Duration(seconds: 15));
      expect(shown.calls.asked, ['status']);

      // A new request: waiting again, with its phrase.
      await tester.tap(find.text('Send a new request'));
      await tester.pump();
      expect(shown.calls.asked.last, 'start https://vault.example.test $email');
      expect(find.text(phrase), findsOneWidget);
      expect(find.text('Sign-in denied'), findsNothing);

      // Or the master password: the request is given up.
      await tester.tap(usePassword);
      await tester.pump();
      expect(shown.calls.asked.last, 'giveUp');
      expect(shown.heard, ['password']);
    },
  );

  testWidgets(
    'expiry, a key that doesn\'t work, and an approval the server won\'t take say so',
    (tester) async {
      for (final (status, title) in [
        ('expired', 'This request expired'),
        ('badKey', 'This approval didn\'t work'),
        ('unusable', 'This approval can\'t be used here'),
      ]) {
        final shown = await show(
          tester,
          asked(),
          statuses: [asked(status: status)],
        );
        await tester.pump(const Duration(seconds: 5));
        expect(find.text(title), findsOneWidget, reason: status);
        expect(find.text('Send a new request'), findsOneWidget);
        expect(find.text('Use master password instead'), findsOneWidget);
        await tester.pump(const Duration(seconds: 10));
        expect(shown.calls.asked, ['status'], reason: status);
      }
    },
  );

  testWidgets('two-step login follows the approval', (tester) async {
    final shown = await show(
      tester,
      asked(),
      statuses: [
        {
          'ok': true,
          'step': 'twoFactor',
          'providers': [0],
        },
      ],
    );
    await tester.pump(const Duration(seconds: 5));
    expect(shown.heard, ['twoStep']);
  });

  testWidgets("a server that can't be reached is tried again", (tester) async {
    final shown = await show(
      tester,
      asked(),
      statuses: [
        {
          'ok': false,
          'error': "PassBundle couldn't connect to vault.example.test.",
          'offline': true,
          'asking': true,
        },
        done,
      ],
    );
    await tester.pump(const Duration(seconds: 5));
    expect(
      find.text("Couldn't reach your server. PassBundle keeps trying."),
      findsOneWidget,
    );
    expect(find.text(phrase), findsOneWidget);
    await tester.pump(const Duration(seconds: 5));
    expect(shown.heard, ['signedIn']);
  });

  testWidgets(
    'a refusal of the sign-in goes back to the master password with its words, as a password sign-in shows them',
    (tester) async {
      const sso =
          'Your server said: Your organization requires you to sign in with single sign-on.';
      await signInScreen(
        tester,
        statuses: [
          {'ok': false, 'error': sso, 'asking': false},
        ],
      );
      await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
      await tester.tap(withDevice);
      await tester.pump();
      await tester.pump();
      expect(shownPhrase, findsOneWidget);
      await tester.pump(const Duration(seconds: 5));
      expect(shownPhrase, findsNothing);
      expect(find.widgetWithText(TextField, 'Master password'), findsOneWidget);
      expect(find.text(sso), findsOneWidget);
    },
  );

  testWidgets(
    'a device that can\'t ask is told so under the master password, and nothing waits',
    (tester) async {
      const notHere =
          'Sign in with a device works on a device that has signed in to this account before. Use your master password this time.';
      final calls = await signInScreen(
        tester,
        starts: [
          {'ok': false, 'error': notHere, 'asking': false},
        ],
      );
      await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
      await tester.tap(withDevice);
      await tester.pump();
      await tester.pump();
      expect(find.text(notHere), findsOneWidget);
      expect(shownPhrase, findsNothing);
      await tester.pump(const Duration(seconds: 10));
      expect(calls.asked.length, 1);
    },
  );

  testWidgets(
    '"Use master password instead" gives the request up and goes back to the master password',
    (tester) async {
      final calls = await signInScreen(tester);
      await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
      await tester.tap(withDevice);
      await tester.pump();
      await tester.pump();
      expect(shownPhrase, findsOneWidget);
      await tester.tap(usePassword);
      await tester.pump();
      expect(calls.asked.last, 'giveUp');
      expect(shownPhrase, findsNothing);
      expect(find.widgetWithText(TextField, 'Master password'), findsOneWidget);
      // The email stays, for signing in with the master password.
      expect(find.text(email), findsOneWidget);
      // Given up: it stops asking.
      final before = calls.asked.length;
      await tester.pump(const Duration(seconds: 15));
      expect(calls.asked.length, before);
    },
  );

  for (final phone in [false, true]) {
    final where = phone ? 'phone' : 'desktop';
    testWidgets('waiting and ended both fit on a $where', (tester) async {
      await show(
        tester,
        asked(),
        statuses: [asked(status: 'denied')],
        phone: phone,
      );
      expect(tester.takeException(), isNull);
      for (final key in [shownPhrase, resend, usePassword]) {
        expect(key.hitTestable(), findsOneWidget, reason: '$key on a $where');
      }
      // The phrase fits the screen's width, wrapped if it must be.
      final screen = tester.getRect(find.byType(DeviceSignInScreen));
      final drawn = tester.getRect(shownPhrase);
      expect(drawn.left, greaterThanOrEqualTo(screen.left));
      expect(drawn.right, lessThanOrEqualTo(screen.right));
      await tester.pump(const Duration(seconds: 5));
      expect(find.text('Sign-in denied'), findsOneWidget);
      expect(tester.takeException(), isNull);
      for (final key in [resend, usePassword]) {
        expect(key.hitTestable(), findsOneWidget, reason: '$key on a $where');
      }
    });
  }
}
