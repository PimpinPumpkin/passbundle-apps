// Approving a sign-in from another device, as the app draws and asks it
// (sign_in_requests.dart), with a stand-in for the core: the prompt shows the
// fingerprint phrase first, with the device, its address, and the time left;
// approving asks the person to confirm it's them first where the device says
// so, and sends nothing if they don't; denying asks nothing; an expired
// request closes itself, and one answered elsewhere too; each device is
// prompted for once every 15 minutes, and a request it sends again takes the
// open prompt's place, with Approve held a moment, only when its identifier,
// type, and address are all the same (otherwise the prompt closes and the new
// one prompts on its own); and Settings lists the waiting ones, and asks to
// confirm before turning confirming off; and another account's request (with
// several accounts signed in) names that account, and is confirmed and
// answered for it. The core's side (the phrase, the key, refusing an
// approval nobody confirmed) has its own tests, against a real server, and
// the meta repo's end-to-end test approves a real web vault's sign-in.
//
// And a sign-in on a new device that the new-device check holds: the
// prompt shows the device, where it's from (with the location's credit),
// and when; Approve waits for two digits and sends them, asking no
// confirmation (no key goes); a wrong number says it denied the sign-in;
// Deny denies; "Not now" brings it back after 2 minutes; one answered
// elsewhere closes; and Settings lists them beside the requests.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/sign_in_requests.dart';
import 'package:passbundle/theme.dart';

import 'support.dart';

const phrase = 'caretaker-uncaring-constrain-rind-dove';

Map<String, dynamic> waiting(
  String id, {
  int age = 30,
  int left = 870,
  String? deviceId,
  String deviceType = 'Chrome',
  String ip = '203.0.113.7',
  String fingerprint = phrase,
}) => {
  'id': id,
  'deviceId': deviceId ?? 'device-$id',
  'deviceType': deviceType,
  'device': 'Web vault in Chrome',
  'ip': ip,
  'fingerprint': fingerprint,
  'ageSeconds': age,
  'secondsLeft': left,
};

/// A sign-in on a new device, as the core lists it for the approving side.
Map<String, dynamic> checkWaiting(
  String id, {
  String? location = 'Berlin, Germany',
  String ip = '203.0.113.7',
  int age = 20,
  int left = 580,
  String? emailAfter,
}) => {
  'id': id,
  'device': 'Web vault in Firefox',
  'deviceType': 10,
  'ip': ip,
  'location': location,
  'ageSeconds': age,
  'secondsLeft': left,
  'emailAfter': emailAfter,
  'locationSource': {
    'name': 'IP Geolocation by DB-IP',
    'url': 'https://db-ip.com',
  },
};

class StandIn implements SignInRequestSource {
  List<Map<String, dynamic>> requests = [];
  List<Map<String, dynamic>> checks = [];

  /// What approving a check answers: "approved" or "wrongNumber".
  String checkAnswer = 'approved';
  final checksAnswered = <(String, String?, String?)>[];
  final answered = <(String, String, bool)>[];
  int confirms = 0;
  bool confirmed = true;

  /// The core asks to confirm again this many times (the prompt sat open).
  int confirmAgain = 0;

  @override
  bool confirmApprovals = true;
  final confirmChanges = <bool>[];

  @override
  Answer setConfirmApprovals(bool on) {
    confirmChanges.add(on);
    confirmApprovals = on;
    return Answer({'ok': true});
  }

  @override
  Future<Answer> list() async =>
      Answer({'ok': true, 'requests': requests, 'checks': checks});

  @override
  Future<Answer> answerCheck(
    String id,
    String? number, [
    String? account,
  ]) async {
    checksAnswered.add((id, number, account));
    return Answer({
      'ok': true,
      'answered': number == null ? 'denied' : checkAnswer,
    });
  }

  /// The accounts each answer and confirmation were for.
  final answeredFor = <String?>[];
  final confirmedFor = <String?>[];

  @override
  Future<Answer> answer(
    String id,
    String fingerprint,
    bool approve, [
    String? account,
  ]) async {
    answeredFor.add(account);
    if (approve && confirmAgain > 0) {
      confirmAgain--;
      return Answer({
        'ok': false,
        'error': "Confirm it's you first.",
        'confirmFirst': true,
      });
    }
    answered.add((id, fingerprint, approve));
    return Answer({'ok': true});
  }

  @override
  Future<bool> confirm(BuildContext context, [String? account]) async {
    confirms++;
    confirmedFor.add(account);
    return confirmed;
  }
}

/// A screen with a button that reviews `request`, as the vault and Settings do.
Future<SignInRequests> review(
  WidgetTester tester,
  StandIn source,
  Map<String, dynamic> request, {
  SignInRequests? requests,
}) async {
  final list = requests ?? SignInRequests(source);
  await tester.pumpWidget(
    MaterialApp(
      theme: passBundleTheme(Brightness.light),
      home: Scaffold(
        body: Builder(
          builder: (context) => TextButton(
            onPressed: () => reviewSignInRequest(
              context,
              SignInRequest(request),
              email: 'dana@example.test',
              requests: list,
            ),
            child: const Text('Open'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open'));
  await tester.pumpAndSettle();
  return list;
}

/// A screen with a button that reviews a new device's `check`.
Future<SignInRequests> reviewCheck(
  WidgetTester tester,
  StandIn source,
  Map<String, dynamic> check, {
  SignInRequests? requests,
}) async {
  final list = requests ?? SignInRequests(source);
  await tester.pumpWidget(
    MaterialApp(
      theme: passBundleTheme(Brightness.light),
      home: Scaffold(
        body: Builder(
          builder: (context) => TextButton(
            onPressed: () => reviewNewDeviceCheck(
              context,
              NewDeviceCheck(check),
              email: 'dana@example.test',
              requests: list,
            ),
            child: const Text('Open'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open'));
  await tester.pumpAndSettle();
  return list;
}

void main() {
  setUpAll(loadFonts);
  tearDown(() => signInClock = DateTime.now);

  testWidgets(
    'a new device\'s sign-in shows where it\'s from, and approves with the number typed, without confirming first',
    (tester) async {
      final source = StandIn();
      await reviewCheck(tester, source, checkWaiting('c1'));
      expect(find.text('Approve this sign-in?'), findsOneWidget);
      expect(
        find.text(
          "Someone is signing in to dana@example.test on a new device. Approve only if it's you, on a device in front of you.",
        ),
        findsOneWidget,
      );
      expect(find.text('Web vault in Firefox'), findsOneWidget);
      expect(find.text('Berlin, Germany (203.0.113.7)'), findsOneWidget);
      expect(find.text('Just now'), findsOneWidget);
      // The location's credit, linked.
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('location-source')),
          matching: find.text('IP Geolocation by DB-IP'),
        ),
        findsOneWidget,
      );
      expect(find.text('Number shown on the new device'), findsOneWidget);
      expect(
        find.text('Two digits. A wrong number denies the sign-in.'),
        findsOneWidget,
      );
      expect(find.text('Not now'), findsOneWidget);

      // Approve waits for two digits.
      final field = find.byKey(const ValueKey('new-device-check-number'));
      await tester.enterText(field, '4');
      await tester.pump();
      await tester.tap(find.text('Approve'));
      await tester.pump();
      expect(source.checksAnswered, isEmpty);
      await tester.enterText(field, '47');
      await tester.pump();
      await tester.tap(find.text('Approve'));
      await tester.pump();
      await tester.pumpAndSettle();
      expect(source.checksAnswered, [('c1', '47', null)]);
      // No key goes, so nothing to confirm.
      expect(source.confirms, 0);
      expect(find.text('Approve this sign-in?'), findsNothing);
      expect(
        find.text('Approved. Web vault in Firefox is signing in.'),
        findsOneWidget,
      );
    },
  );

  testWidgets('a wrong number says it denied the sign-in', (tester) async {
    final source = StandIn()..checkAnswer = 'wrongNumber';
    await reviewCheck(tester, source, checkWaiting('c1'));
    await tester.enterText(
      find.byKey(const ValueKey('new-device-check-number')),
      '12',
    );
    // Approve comes on once two digits are in.
    await tester.pump();
    await tester.tap(find.text('Approve'));
    await tester.pumpAndSettle();
    expect(source.checksAnswered, [('c1', '12', null)]);
    expect(
      find.text(
        "That wasn't the number on the new device, so the sign-in was denied. If it was you, sign in again there.",
      ),
      findsOneWidget,
    );
  });

  testWidgets(
    'Deny sends no number; an unknown location is said so, without a credit',
    (tester) async {
      final source = StandIn();
      await reviewCheck(
        tester,
        source,
        checkWaiting(
          'c1',
          location: null,
          ip: '10.0.0.2',
          emailAfter: '2026-11-09T12:00:00.000000Z',
        ),
      );
      expect(find.text('Unknown location (10.0.0.2)'), findsOneWidget);
      expect(find.byKey(const ValueKey('location-source')), findsNothing);
      // Someone asked for a code by email instead: said, to deny.
      expect(
        find.textContaining('Someone asked for a code by email'),
        findsOneWidget,
      );
      await tester.tap(find.text('Deny'));
      await tester.pumpAndSettle();
      expect(source.checksAnswered, [('c1', null, null)]);
      expect(find.text("Denied. That sign-in won't go ahead."), findsOneWidget);
    },
  );

  testWidgets(
    'another account\'s check names it and is answered for it; one answered elsewhere closes',
    (tester) async {
      final home = 'b' * 32;
      final source = StandIn()
        ..checks = [
          {
            ...checkWaiting('c1'),
            'account': home,
            'email': 'dana@home.example.test',
          },
        ];
      final list = SignInRequests(source)..start();
      await tester.runAsync(list.check);
      await reviewCheck(tester, source, source.checks.first, requests: list);
      expect(
        find.textContaining('signing in to dana@home.example.test'),
        findsOneWidget,
      );
      await tester.enterText(
        find.byKey(const ValueKey('new-device-check-number')),
        '47',
      );
      await tester.pump();
      await tester.tap(find.text('Approve'));
      await tester.pumpAndSettle();
      expect(source.checksAnswered, [('c1', '47', home)]);
      // The note it approved goes after a few seconds.
      await tester.pump(const Duration(seconds: 6));
      await tester.pumpAndSettle();

      source.checks = [checkWaiting('c2')];
      await tester.runAsync(list.check);
      await reviewCheck(tester, source, checkWaiting('c2'), requests: list);
      source.checks = [];
      await tester.runAsync(list.check);
      await tester.pumpAndSettle();
      expect(find.text('Approve this sign-in?'), findsNothing);
      expect(
        find.text('That sign-in was answered on another device, or it ended.'),
        findsOneWidget,
      );
      list.stop();
    },
  );

  test('"Not now" brings a check back after 2 minutes', () async {
    var now = DateTime(2026, 11, 8, 12);
    signInClock = () => now;
    final source = StandIn()..checks = [checkWaiting('c1')];
    final list = SignInRequests(source)..start();
    await list.check();
    expect(list.unpromptedCheck()?.id, 'c1');
    expect(list.unpromptedCheck(), isNull);
    now = now.add(const Duration(minutes: 1));
    await list.check();
    expect(list.unpromptedCheck(), isNull);
    now = now.add(const Duration(minutes: 1));
    await list.check();
    expect(list.unpromptedCheck()?.id, 'c1');
    // Requests to sign in with a device are prompted for as before.
    expect(list.unprompted(), isNull);
    list.stop();
    expect(list.checks, isEmpty);
  });

  testWidgets('Settings lists a new device\'s check beside the requests', (
    tester,
  ) async {
    final source = StandIn()
      ..requests = [waiting('r1')]
      ..checks = [checkWaiting('c1')];
    final list = SignInRequests(source)..start();
    await tester.pumpWidget(
      MaterialApp(
        theme: passBundleTheme(Brightness.light),
        home: Scaffold(
          body: SingleChildScrollView(
            child: SignInRequestsSettings(requests: list),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('new-device-check-c1')), findsOneWidget);
    expect(find.byKey(const ValueKey('sign-in-request-r1')), findsOneWidget);
    expect(
      find.textContaining('New device · Berlin, Germany (203.0.113.7)'),
      findsOneWidget,
    );
    await tester.tap(
      find.descendant(
        of: find.byKey(const ValueKey('new-device-check-c1')),
        matching: find.text('Review'),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Deny'));
    await tester.pumpAndSettle();
    expect(source.checksAnswered, [('c1', null, null)]);
    expect(find.byKey(const ValueKey('new-device-check-c1')), findsNothing);
    list.stop();
  });

  testWidgets(
    'the prompt shows the phrase and the request, and approving asks to confirm first',
    (tester) async {
      final source = StandIn();
      await review(tester, source, waiting('r1'));
      expect(find.text('Approve sign-in?'), findsOneWidget);
      expect(
        find.text(
          'Web vault in Chrome is asking to sign in as dana@example.test.',
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('sign-in-fingerprint')),
          matching: find.text(phrase),
        ),
        findsOneWidget,
      );
      expect(find.text('203.0.113.7'), findsOneWidget);
      expect(find.text('Just now'), findsOneWidget);
      expect(find.text('14 minutes'), findsOneWidget);

      await tester.tap(find.text('Approve'));
      await tester.pumpAndSettle();
      expect(source.confirms, 1);
      expect(source.answered, [('r1', phrase, true)]);
      expect(find.text('Approve sign-in?'), findsNothing);
      expect(
        find.text('Approved. Web vault in Chrome is signing in.'),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'another account\'s request names that account, and is confirmed and answered for it',
    (tester) async {
      final source = StandIn();
      final home = 'b' * 32;
      await review(tester, source, {
        ...waiting('r1'),
        'account': home,
        'email': 'dana@home.example.test',
      });
      expect(
        find.text(
          'Web vault in Chrome is asking to sign in as dana@home.example.test.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Approve'));
      await tester.pumpAndSettle();
      expect(source.confirmedFor, [home]);
      expect(source.answeredFor, [home]);
      expect(source.answered, [('r1', phrase, true)]);
    },
  );

  testWidgets('not confirming sends nothing, and the prompt stays', (
    tester,
  ) async {
    final source = StandIn()..confirmed = false;
    await review(tester, source, waiting('r1'));
    await tester.tap(find.text('Approve'));
    await tester.pumpAndSettle();
    expect(source.confirms, 1);
    expect(source.answered, isEmpty);
    expect(find.text('Approve sign-in?'), findsOneWidget);
  });

  testWidgets('a confirmation that went stale is asked for again', (
    tester,
  ) async {
    final source = StandIn()..confirmAgain = 1;
    await review(tester, source, waiting('r1'));
    await tester.tap(find.text('Approve'));
    await tester.pumpAndSettle();
    expect(source.confirms, 2);
    expect(source.answered, [('r1', phrase, true)]);
  });

  testWidgets(
    'with confirming turned off, approving asks nothing; denying never does',
    (tester) async {
      final source = StandIn()..confirmApprovals = false;
      await review(tester, source, waiting('r1'));
      await tester.tap(find.text('Approve'));
      await tester.pumpAndSettle();
      expect(source.confirms, 0);
      expect(source.answered, [('r1', phrase, true)]);

      // The note it approved goes after a few seconds.
      await tester.pump(const Duration(seconds: 5));
      await tester.pumpAndSettle();
      source.confirmApprovals = true;
      await review(tester, source, waiting('r2'));
      await tester.tap(find.text('Deny'));
      await tester.pumpAndSettle();
      expect(source.confirms, 0);
      expect(source.answered.last, ('r2', phrase, false));
      expect(find.text("Denied. That sign-in won't go ahead."), findsOneWidget);
    },
  );

  testWidgets('a request whose time is up closes by itself', (tester) async {
    var now = DateTime(2026, 10, 26, 12);
    signInClock = () => now;
    final source = StandIn();
    await review(tester, source, waiting('r1', left: 90));
    expect(find.text('1 minute'), findsOneWidget);
    now = now.add(const Duration(seconds: 89));
    await tester.pump(const Duration(seconds: 1));
    expect(find.text('Under a minute'), findsOneWidget);
    now = now.add(const Duration(seconds: 2));
    await tester.pump(const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(find.text('Approve sign-in?'), findsNothing);
    expect(find.text('That sign-in request expired.'), findsOneWidget);
    expect(source.answered, isEmpty);
  });

  testWidgets(
    'each new request is prompted for once, and Settings lists the waiting ones',
    (tester) async {
      final source = StandIn()..requests = [waiting('r1'), waiting('r2')];
      final list = SignInRequests(source)..start();
      await tester.pumpWidget(
        MaterialApp(
          theme: passBundleTheme(Brightness.light),
          home: Scaffold(
            body: SingleChildScrollView(
              child: SignInRequestsSettings(requests: list),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(list.unprompted()?.id, 'r1');
      expect(list.unprompted()?.id, 'r2');
      expect(list.unprompted(), isNull);
      expect(find.byKey(const ValueKey('sign-in-request-r1')), findsOneWidget);
      expect(find.byKey(const ValueKey('sign-in-request-r2')), findsOneWidget);

      await tester.tap(
        find.descendant(
          of: find.byKey(const ValueKey('sign-in-request-r2')),
          matching: find.text('Review'),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Deny'));
      await tester.pumpAndSettle();
      expect(source.answered, [('r2', phrase, false)]);
      // Answered here, it's gone from the list at once.
      expect(find.byKey(const ValueKey('sign-in-request-r2')), findsNothing);

      // Turning confirming off asks the person to confirm it's them first:
      // not confirmed, it stays on.
      source.confirmed = false;
      await tester.tap(find.byKey(const ValueKey('confirm-approvals')));
      await tester.pumpAndSettle();
      expect(source.confirms, 1);
      expect(source.confirmChanges, isEmpty);
      expect(source.confirmApprovals, isTrue);
      source.confirmed = true;
      await tester.tap(find.byKey(const ValueKey('confirm-approvals')));
      await tester.pumpAndSettle();
      expect(source.confirms, 2);
      expect(source.confirmChanges, [false]);
      expect(source.confirmApprovals, isFalse);
      // Turning it back on asks nothing.
      await tester.tap(find.byKey(const ValueKey('confirm-approvals')));
      await tester.pumpAndSettle();
      expect(source.confirms, 2);
      expect(source.confirmChanges, [false, true]);

      // Nothing waiting: it says so.
      source.requests = [];
      await list.check();
      await tester.pumpAndSettle();
      expect(find.text('None waiting'), findsOneWidget);
      list.stop();
    },
  );

  test('each device is prompted for once every 15 minutes', () async {
    var now = DateTime(2026, 10, 26, 12);
    signInClock = () => now;
    final source = StandIn()
      ..requests = [
        waiting('a1', deviceId: 'laptop'),
        waiting('b1', deviceId: 'phone'),
      ];
    final list = SignInRequests(source)..start();
    await list.check();
    expect(list.unprompted()?.id, 'a1');
    expect(list.unprompted()?.id, 'b1');
    expect(list.unprompted(), isNull);
    // The laptop sends it again ("Resend"): no new prompt...
    source.requests = [waiting('a2', deviceId: 'laptop')];
    await list.check();
    expect(list.unprompted(), isNull);
    // ...nor for one more a minute later; after 15 minutes, a new one.
    now = now.add(const Duration(minutes: 1));
    source.requests = [waiting('a3', deviceId: 'laptop')];
    await list.check();
    expect(list.unprompted(), isNull);
    now = now.add(const Duration(minutes: 15));
    source.requests = [waiting('a4', deviceId: 'laptop')];
    await list.check();
    expect(list.unprompted()?.id, 'a4');
    list.stop();
  });

  testWidgets(
    'a request sent again takes the open prompt\'s place, with Approve held a moment',
    (tester) async {
      var now = DateTime(2026, 10, 26, 12);
      signInClock = () => now;
      const resent = 'amply-blizzard-dove-ladle-sprout';
      final source = StandIn()..requests = [waiting('a1', deviceId: 'laptop')];
      final list = SignInRequests(source)..start();
      await tester.runAsync(list.check);
      await review(
        tester,
        source,
        waiting('a1', deviceId: 'laptop'),
        requests: list,
      );
      expect(find.text(phrase), findsOneWidget);

      source.requests = [
        waiting('a2', deviceId: 'laptop', fingerprint: resent),
      ];
      await tester.runAsync(list.check);
      await tester.pump();
      expect(find.text(resent), findsOneWidget);
      expect(find.byKey(const ValueKey('sign-in-replaced')), findsOneWidget);
      // Held: tapping Approve does nothing yet.
      await tester.tap(find.text('Approve'));
      await tester.pump();
      expect(source.confirms, 0);
      now = now.add(const Duration(seconds: 4));
      await tester.pump(const Duration(seconds: 1));
      await tester.tap(find.text('Approve'));
      await tester.pumpAndSettle();
      expect(source.answered, [('a2', resent, true)]);
      list.stop();
    },
  );

  testWidgets('a request answered elsewhere closes its prompt', (tester) async {
    final source = StandIn()..requests = [waiting('a1')];
    final list = SignInRequests(source)..start();
    await tester.runAsync(list.check);
    await review(tester, source, waiting('a1'), requests: list);
    expect(find.text('Approve sign-in?'), findsOneWidget);
    source.requests = [];
    await tester.runAsync(list.check);
    await tester.pumpAndSettle();
    expect(find.text('Approve sign-in?'), findsNothing);
    expect(
      find.text('That sign-in request was answered elsewhere, or expired.'),
      findsOneWidget,
    );
    expect(source.answered, isEmpty);
    list.stop();
  });

  for (final (what, changed) in [
    ('another address', {'ip': '198.51.100.9'}),
    ('another type', {'deviceType': 'Firefox'}),
  ]) {
    testWidgets(
      'a new request from the same device identifier but $what closes the prompt and prompts on its own',
      (tester) async {
        const other = 'amply-blizzard-dove-ladle-sprout';
        final source = StandIn()
          ..requests = [waiting('a1', deviceId: 'laptop')];
        final list = SignInRequests(source)..start();
        await tester.runAsync(list.check);
        expect(list.unprompted()?.id, 'a1');
        await review(
          tester,
          source,
          waiting('a1', deviceId: 'laptop'),
          requests: list,
        );
        source.requests = [
          {
            ...waiting('a2', deviceId: 'laptop', fingerprint: other),
            ...changed,
          },
        ];
        await tester.runAsync(list.check);
        await tester.pumpAndSettle();
        // Not swapped in: the open prompt is gone, never showing its phrase...
        expect(find.text(other), findsNothing);
        expect(find.text('Approve sign-in?'), findsNothing);
        expect(
          find.text('That sign-in request was answered elsewhere, or expired.'),
          findsOneWidget,
        );
        // ...and it's prompted for on its own.
        expect(list.unprompted()?.id, 'a2');
        expect(source.answered, isEmpty);
        list.stop();
      },
    );
  }
}
