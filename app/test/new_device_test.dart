// Signing in on a device the account has never used (new_device.dart), with
// a stand-in for the core's calls: the number shown large, asked about every
// 3 seconds until another device approves it; a code by email straight away
// when no device can approve (with what it costs said beside it), or after
// a day's wait when one could; and denied and ended sign-ins, with Start
// over. The core's side (holding the sign-in, the token, the device token
// kept) has its own tests, against a real server.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/new_device.dart';

import 'support.dart';

/// A check as the core answers with it (core/src/ffi.rs, `new_device_check`).
Map<String, dynamic> held({
  String status = 'waiting',
  String email = 'later',
  String? deniedWhy,
  bool codeSent = false,
  String? emailAfter,
}) => {
  'ok': true,
  'step': 'newDevice',
  'check': {
    'id': '5f0c6f1e-6f3b-4a43-9a52-0d3f1c2b7e11',
    'number': '47',
    'status': status,
    'deniedWhy': deniedWhy,
    'expiresAt': '2026-11-08T12:10:00.000000Z',
    'approvers': email != 'now',
    'email': email,
    'emailAfter': emailAfter,
    'codeSent': codeSent,
    'emailTo': codeSent ? 'a***@example.test' : null,
  },
};

const done = {'ok': true, 'step': 'done'};

/// The core's calls, answered in order, and what was asked.
class FakeCalls extends NewDeviceCalls {
  FakeCalls({this.statuses = const [], this.emails = const [], this.codes = const []});

  final List<Map<String, dynamic>> statuses;
  final List<Map<String, dynamic>> emails;
  final List<Map<String, dynamic>> codes;
  final asked = <String>[];

  Answer _next(List<Map<String, dynamic>> answers, int at) =>
      Answer(at < answers.length ? answers[at] : answers.last);

  @override
  Future<Answer> status() async {
    asked.add('status');
    return _next(statuses, asked.where((a) => a == 'status').length - 1);
  }

  @override
  Future<Answer> email() async {
    asked.add('email');
    return _next(emails, asked.where((a) => a == 'email').length - 1);
  }

  @override
  Future<Answer> code(String code) async {
    asked.add('code $code');
    return _next(codes, asked.where((a) => a.startsWith('code')).length - 1);
  }
}

void main() {
  setUpAll(loadFonts);

  Future<({FakeCalls calls, List<String> heard})> show(
    WidgetTester tester,
    Map<String, dynamic> answer, {
    List<Map<String, dynamic>> statuses = const [],
    List<Map<String, dynamic>> emails = const [],
    List<Map<String, dynamic>> codes = const [],
  }) async {
    tester.view.physicalSize = const Size(900, 1600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final calls = FakeCalls(
      statuses: statuses.isEmpty ? [answer] : statuses,
      emails: emails.isEmpty ? [answer] : emails,
      codes: codes.isEmpty ? [answer] : codes,
    );
    final heard = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        home: NewDeviceScreen(
          key: UniqueKey(),
          answer: answer,
          calls: calls,
          onSignedIn: () => heard.add('signedIn'),
          onStartOver: () => heard.add('startOver'),
          onTwoStep: (_) => heard.add('twoStep'),
        ),
      ),
    );
    return (calls: calls, heard: heard);
  }

  testWidgets(
    'the number shows large, and the sign-in finishes once another device approves it',
    (tester) async {
      final shown = await show(
        tester,
        held(),
        statuses: [held(), held(status: 'approved'), done],
      );
      expect(find.text('Approve this sign-in'), findsOneWidget);
      expect(find.text('This device is new to your account.'), findsOneWidget);
      expect(
        find.text(
          "On a device where you're already signed in, open your vault and type this number:",
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('new-device-number')),
          matching: find.text('47'),
        ),
        findsOneWidget,
      );
      expect(
        find.text('Waiting for approval. This works for 10 minutes.'),
        findsOneWidget,
      );
      // Nothing is asked before the first 3 seconds are up.
      await tester.pump(const Duration(seconds: 2));
      expect(shown.calls.asked, isEmpty);
      await tester.pump(const Duration(seconds: 1));
      expect(shown.calls.asked, ['status']);
      expect(shown.heard, isEmpty);
      await tester.pump(const Duration(seconds: 3));
      await tester.pump(const Duration(seconds: 3));
      expect(shown.calls.asked, ['status', 'status', 'status']);
      expect(shown.heard, ['signedIn']);
      // Signed in: it stops asking.
      await tester.pump(const Duration(seconds: 6));
      expect(shown.calls.asked.length, 3);
    },
  );

  testWidgets(
    'with no device to approve it, a code by email, said to be less safe, and a wrong one said so',
    (tester) async {
      final shown = await show(
        tester,
        held(email: 'now'),
        emails: [held(email: 'now', codeSent: true)],
        codes: [
          {
            'ok': false,
            'error': "That code didn't work. Check the email and try again.",
          },
          done,
        ],
      );
      expect(find.text('Email me a code'), findsOneWidget);
      expect(find.text(emailIsWeaker), findsOneWidget);
      expect(find.text("Can't use your other devices?"), findsNothing);

      await tester.tap(find.text('Email me a code'));
      await tester.pump();
      expect(shown.calls.asked, ['email']);
      expect(find.text('Your server emailed a code to a***@example.test.'), findsOneWidget);
      expect(find.text('Code from the email'), findsOneWidget);
      expect(find.text('Send another code'), findsOneWidget);

      await tester.enterText(
        find.byKey(const ValueKey('new-device-code')),
        '1234 5678',
      );
      await tester.tap(find.text('Continue'));
      await tester.pump();
      expect(shown.calls.asked.last, 'code 1234 5678');
      expect(
        find.text("That code didn't work. Check the email and try again."),
        findsOneWidget,
      );
      expect(shown.heard, isEmpty);

      await tester.tap(find.text('Send another code'));
      await tester.pump();
      expect(shown.calls.asked.last, 'email');

      await tester.enterText(
        find.byKey(const ValueKey('new-device-code')),
        '87654321',
      );
      await tester.tap(find.text('Continue'));
      await tester.pump();
      expect(shown.calls.asked.last, 'code 87654321');
      expect(shown.heard, ['signedIn']);
    },
  );

  testWidgets(
    "with devices that could approve it, a code by email only after a day's wait",
    (tester) async {
      final shown = await show(
        tester,
        held(),
        emails: [
          held(email: 'waiting', emailAfter: '2026-11-09T12:00:00.000000Z'),
        ],
      );
      expect(find.text('Email me a code'), findsNothing);
      await tester.tap(find.text("Can't use your other devices?"));
      await tester.pump();
      expect(find.textContaining('after a 24-hour wait'), findsOneWidget);
      expect(find.text(emailIsWeaker), findsOneWidget);

      await tester.tap(find.text('Email me a code in 24 hours'));
      await tester.pump();
      expect(shown.calls.asked, ['email']);
      expect(
        find.textContaining('You asked for a code by email. After '),
        findsOneWidget,
      );
      expect(
        find.text(
          'Your other devices can still approve or deny this sign-in meanwhile.',
        ),
        findsOneWidget,
      );
      // The number still shows: another device can still approve it.
      expect(find.text('47'), findsOneWidget);
      expect(find.text('Waiting for approval.'), findsOneWidget);
    },
  );

  testWidgets('a wrong number typed elsewhere denies it, and Start over goes back', (
    tester,
  ) async {
    final shown = await show(
      tester,
      held(),
      statuses: [held(status: 'denied', deniedWhy: 'number')],
    );
    await tester.pump(const Duration(seconds: 3));
    expect(find.text('Sign-in denied'), findsOneWidget);
    expect(
      find.text(
        "The number typed on your other device didn't match, so this sign-in was denied.",
      ),
      findsOneWidget,
    );
    expect(find.text('47'), findsNothing);
    // Denied: it stops asking.
    await tester.pump(const Duration(seconds: 9));
    expect(shown.calls.asked, ['status']);
    await tester.tap(find.text('Start over'));
    await tester.pump();
    expect(shown.heard, ['startOver']);
  });

  testWidgets('denied from another device, and a sign-in that ended', (
    tester,
  ) async {
    await show(tester, held(status: 'denied', deniedWhy: 'denied'));
    expect(
      find.text('This sign-in was denied from another of your devices.'),
      findsOneWidget,
    );
    await show(tester, held(status: 'ended'));
    expect(find.text('This sign-in ended'), findsOneWidget);
    expect(find.text('Start over'), findsOneWidget);
  });

  testWidgets('two-step login asked for instead goes to its screen', (
    tester,
  ) async {
    final shown = await show(
      tester,
      held(),
      statuses: [
        {
          'ok': true,
          'step': 'twoFactor',
          'providers': [0],
        },
      ],
    );
    await tester.pump(const Duration(seconds: 3));
    expect(shown.heard, ['twoStep']);
  });

  testWidgets("a server that can't be reached is tried again", (tester) async {
    final shown = await show(
      tester,
      held(),
      statuses: [
        {
          'ok': false,
          'error': "PassBundle couldn't connect to vault.example.test.",
          'offline': true,
        },
        done,
      ],
    );
    await tester.pump(const Duration(seconds: 3));
    expect(
      find.text("Couldn't reach your server. PassBundle keeps trying."),
      findsOneWidget,
    );
    await tester.pump(const Duration(seconds: 3));
    expect(shown.heard, ['signedIn']);
  });
}
