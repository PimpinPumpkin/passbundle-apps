// Two-step login's second step (two_step.dart): the strongest method offered
// first, every other way (a recovery code always), and what each says and
// sends, with a stand-in for the core's calls. two_step_server_test.dart
// goes through the real core to a stand-in server.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/two_step.dart';

import 'support.dart';

/// The core's calls, answered as each test says, and what was asked.
class FakeCalls extends TwoStepCalls {
  FakeCalls({this.answers = const [], this.links = const []});

  /// Answers, in order, for finish, sendEmail, securityKey, duoStart, and duoFinish.
  final List<Map<String, dynamic>> answers;

  /// The links Duo comes back with, in order (null: given up).
  final List<String?> links;
  final asked = <String>[];
  var _answer = 0;
  var _link = 0;
  bool listeningForYubikey = false;

  Future<Answer> _next(String what) async {
    asked.add(what);
    return Answer(
      _answer < answers.length ? answers[_answer++] : {'ok': true, 'step': 'done'},
    );
  }

  @override
  Future<Answer> finish(Method method, String code, bool remember) =>
      _next('finish ${method.name} $code${remember ? ' remember' : ''}');

  @override
  Future<Answer> sendEmail() => _next('sendEmail');

  @override
  Future<Answer> securityKey(bool remember) =>
      _next('securityKey${remember ? ' remember' : ''}');

  @override
  void cancelSecurityKey() => asked.add('cancelSecurityKey');

  @override
  Future<Answer> duoStart() => _next('duoStart');

  @override
  Future<String?> duoBrowse(String url) async {
    asked.add('duoBrowse $url');
    return _link < links.length ? links[_link++] : null;
  }

  @override
  void cancelDuo() => asked.add('cancelDuo');

  @override
  Future<Answer> duoFinish(String link, bool remember) =>
      _next('duoFinish $link');

  @override
  void yubikeyNfc(bool on, void Function(String code)? onCode) =>
      listeningForYubikey = on;
}

const _otp = 'cccccbhuinjdtrbftdrldjlrgkdeubffnkhjfjifvlbr';

void main() {
  setUpAll(loadFonts);

  Future<({FakeCalls calls, List<String> heard})> show(
    WidgetTester tester,
    Map<String, dynamic> offer, {
    List<Map<String, dynamic>> answers = const [],
    List<String?> links = const [],
  }) async {
    tester.view.physicalSize = const Size(900, 1400);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final calls = FakeCalls(answers: answers, links: links);
    final heard = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        home: TwoStepScreen(
          // A screen of its own each time, as each sign-in's is.
          key: UniqueKey(),
          offer: {'ok': true, 'step': 'twoFactor', ...offer},
          calls: calls,
          onSignedIn: () => heard.add('signedIn'),
          onStartOver: () => heard.add('startOver'),
        ),
      ),
    );
    return (calls: calls, heard: heard);
  }

  Future<void> otherWay(WidgetTester tester, String label) async {
    await tester.tap(find.text('Use another way'));
    await tester.pump();
    await tester.tap(find.widgetWithText(OutlinedButton, label));
    await tester.pump();
  }

  test('the strongest method offered comes first', () {
    Method first(List<int> providers, [String? here = 'usb']) =>
        firstMethod({'providers': providers, 'securityKey': here});
    expect(first([0, 1, 2, 3, 7]), Method.securityKey);
    expect(first([0, 1, 2, 3, 7], null), Method.duo);
    expect(first([1, 3, 0]), Method.yubikey);
    expect(first([1, 0]), Method.authenticator);
    expect(first([1]), Method.email);
    // Only a key this device can't use: it says so.
    expect(first([7], null), Method.securityKey);
    // Methods the apps don't know aren't listed.
    expect(methodsOffered({'providers': [6, 5, 0]}), [Method.authenticator]);
  });

  testWidgets(
    'a security key comes first, and every other way is a tap away, a recovery code included',
    (tester) async {
      final shown = await show(tester, {
        'providers': [0, 1, 2, 3, 7],
        'securityKey': 'usb',
        'email': 'an***@example.test',
      });
      expect(find.text('Two-step login'), findsOneWidget);
      expect(
        find.text('Plug in your security key, then touch it when it blinks.'),
        findsOneWidget,
      );
      expect(find.text('Use security key'), findsOneWidget);
      await tester.tap(find.text('Use another way'));
      await tester.pump();
      for (final label in [
        'Duo',
        'YubiKey',
        'Authenticator app',
        'Email',
        'Recovery code',
      ]) {
        expect(find.widgetWithText(OutlinedButton, label), findsOneWidget);
      }
      expect(find.widgetWithText(OutlinedButton, 'Security key'), findsNothing);
      await tester.tap(find.text('Start over'));
      expect(shown.heard, ['startOver']);
    },
  );

  testWidgets(
    'a security key that wasn\'t touched says so, and the next try signs in; Cancel stops the wait',
    (tester) async {
      final shown = await show(
        tester,
        {
          'providers': [7],
          'securityKey': 'phone',
        },
        answers: [
          {
            'ok': false,
            'error': 'No security key was touched in time. Try again.',
          },
          {'ok': false, 'error': 'Canceled.'},
        ],
      );
      expect(
        find.textContaining('Hold your security key to the back of your phone'),
        findsOneWidget,
      );
      await tester.tap(find.text('Don\'t ask again on this device for a while'));
      await tester.pump();
      await tester.tap(find.text('Use security key'));
      await tester.pump();
      expect(
        find.text('No security key was touched in time. Try again.'),
        findsOneWidget,
      );
      // Giving up isn't an error.
      await tester.tap(find.text('Use security key'));
      await tester.pump();
      expect(find.textContaining('touched in time'), findsNothing);
      await tester.tap(find.text('Use security key'));
      await tester.pump();
      expect(shown.calls.asked, [
        'securityKey remember',
        'securityKey remember',
        'securityKey remember',
      ]);
      expect(shown.heard, ['signedIn']);
    },
  );

  testWidgets(
    'a device that can\'t use a security key says so, and offers the other ways',
    (tester) async {
      await show(tester, {
        'providers': [7],
        'securityKey': null,
      });
      expect(find.text(notHere), findsOneWidget);
      expect(find.text('Use security key'), findsNothing);
      // With nothing else offered, the recovery code is what's left.
      await tester.tap(find.text('Use another way'));
      await tester.pump();
      expect(find.widgetWithText(OutlinedButton, 'Recovery code'), findsOneWidget);
      // And with something else offered, that comes first, with the key marked.
      await show(tester, {
        'providers': [7, 0],
        'securityKey': null,
      });
      expect(
        find.text('Enter the code from your authenticator app.'),
        findsOneWidget,
      );
      await tester.tap(find.text('Use another way'));
      await tester.pump();
      expect(
        find.widgetWithText(
          OutlinedButton,
          'Security key (not on this device yet)',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'Duo opens in the browser and finishes with the link it comes back with; given up, it can be tried again',
    (tester) async {
      const link = 'passbundle://duo-callback?code=c0de&state=s1';
      final shown = await show(
        tester,
        {
          'providers': [2, 0],
          'securityKey': 'windows',
        },
        answers: [
          {'ok': true, 'url': 'https://api-1a2b.duosecurity.com/a'},
          {'ok': true, 'url': 'https://api-1a2b.duosecurity.com/b'},
        ],
        links: [null, link],
      );
      expect(
        find.text('Approve this sign-in with Duo, in your browser.'),
        findsOneWidget,
      );
      await tester.tap(find.text('Continue with Duo'));
      await tester.pump();
      expect(
        find.text('Duo wasn\'t finished. Continue with Duo to try again.'),
        findsOneWidget,
      );
      await tester.tap(find.text('Continue with Duo'));
      await tester.pump();
      expect(shown.calls.asked, [
        'duoStart',
        'duoBrowse https://api-1a2b.duosecurity.com/a',
        'duoStart',
        'duoBrowse https://api-1a2b.duosecurity.com/b',
        'duoFinish $link',
      ]);
      expect(shown.heard, ['signedIn']);
    },
  );

  testWidgets(
    'a YubiKey\'s code goes to the core as the key typed it, and what the core says shows',
    (tester) async {
      final shown = await show(
        tester,
        {
          'providers': [3],
          'securityKey': 'usb',
        },
        answers: [
          {
            'ok': false,
            'error':
                'That YubiKey code didn\'t work. Touch your YubiKey again, or use another way to sign in.',
          },
        ],
      );
      expect(
        find.text('Click in the field, then touch your YubiKey.'),
        findsOneWidget,
      );
      expect(shown.calls.listeningForYubikey, isTrue);
      await tester.enterText(find.widgetWithText(TextField, 'YubiKey code'), _otp);
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(find.textContaining('That YubiKey code didn\'t work.'), findsOneWidget);
      await tester.enterText(find.widgetWithText(TextField, 'YubiKey code'), _otp);
      await tester.tap(find.text('Continue'));
      await tester.pump();
      expect(shown.calls.asked, ['finish yubikey $_otp', 'finish yubikey $_otp']);
      expect(shown.heard, ['signedIn']);
      // Choosing another way stops listening for one held to the phone.
      final again = await show(tester, {
        'providers': [3, 0],
        'securityKey': 'usb',
      });
      expect(again.calls.listeningForYubikey, isTrue);
      await otherWay(tester, 'Authenticator app');
      expect(again.calls.listeningForYubikey, isFalse);
    },
  );

  testWidgets(
    'a recovery code warns that it turns two-step login off, then says it did',
    (tester) async {
      final shown = await show(tester, {
        'providers': [0],
        'securityKey': 'usb',
      });
      await otherWay(tester, 'Recovery code');
      expect(find.text('Enter the recovery code you saved.'), findsOneWidget);
      expect(
        find.text(
          'Signing in with it turns two-step login off for this account.',
        ),
        findsOneWidget,
      );
      // No "don't ask again": there won't be anything to ask.
      expect(
        find.text('Don\'t ask again on this device for a while'),
        findsNothing,
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Recovery code'),
        'ABCD EFGH IJKL',
      );
      await tester.tap(find.text('Continue'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text('Two-step login is off'), findsOneWidget);
      expect(shown.heard, isEmpty);
      await tester.tap(find.text('OK'));
      // The dialog fades out; the screen's button keeps spinning until the
      // vault replaces it, so this can't wait for everything to settle.
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text('Two-step login is off'), findsNothing);
      expect(shown.calls.asked, ['finish recoveryCode ABCD EFGH IJKL']);
      expect(shown.heard, ['signedIn']);
    },
  );

  testWidgets('an email code is sent first, then asked for, at the address shown', (
    tester,
  ) async {
    final shown = await show(tester, {
      'providers': [1, 0],
      'securityKey': 'usb',
      'email': 'an***@example.test',
    });
    // The authenticator app is stronger, so it's first.
    await otherWay(tester, 'Email');
    expect(find.text('Get a code by email at an***@example.test.'), findsOneWidget);
    await tester.tap(find.text('Email me a code'));
    await tester.pump();
    expect(find.text('We emailed a code to an***@example.test.'), findsOneWidget);
    await tester.enterText(find.widgetWithText(TextField, 'Code'), '123456');
    await tester.tap(find.text('Continue'));
    await tester.pump();
    expect(shown.calls.asked, ['sendEmail', 'finish email 123456']);
    expect(shown.heard, ['signedIn']);
  });
}
