// Approving a sign-in from another device, as the app draws and asks it
// (sign_in_requests.dart), with a stand-in for the core: the prompt shows the
// fingerprint phrase first, with the device, its address, and the time left;
// approving asks the person to confirm it's them first where the device says
// so, and sends nothing if they don't; denying asks nothing; an expired
// request closes itself; each new request is prompted for once; and Settings
// lists the waiting ones. The core's side (the phrase, the key, refusing an
// approval nobody confirmed) has its own tests, against a real server, and
// the meta repo's end-to-end test approves a real web vault's sign-in.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/sign_in_requests.dart';
import 'package:passbundle/theme.dart';

import 'support.dart';

const phrase = 'caretaker-uncaring-constrain-rind-dove';

Map<String, dynamic> waiting(String id, {int age = 30, int left = 870}) => {
  'id': id,
  'device': 'Web vault in Chrome',
  'ip': '203.0.113.7',
  'fingerprint': phrase,
  'ageSeconds': age,
  'secondsLeft': left,
};

class StandIn implements SignInRequestSource {
  List<Map<String, dynamic>> requests = [];
  final answered = <(String, String, bool)>[];
  int confirms = 0;
  bool confirmed = true;

  /// The core asks to confirm again this many times (the prompt sat open).
  int confirmAgain = 0;

  @override
  bool confirmApprovals = true;

  @override
  Future<Answer> list() async => Answer({'ok': true, 'requests': requests});

  @override
  Future<Answer> answer(String id, String fingerprint, bool approve) async {
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
  Future<bool> confirm(BuildContext context) async {
    confirms++;
    return confirmed;
  }
}

/// A screen with a button that reviews `request`, as the vault and Settings do.
Future<SignInRequests> review(
  WidgetTester tester,
  StandIn source,
  Map<String, dynamic> request,
) async {
  final list = SignInRequests(source);
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

void main() {
  setUpAll(loadFonts);
  tearDown(() => signInClock = DateTime.now);

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

      // Turning confirming off is this device's choice.
      await tester.tap(find.byKey(const ValueKey('confirm-approvals')));
      await tester.pumpAndSettle();
      expect(source.confirmApprovals, isFalse);

      // Nothing waiting: it says so.
      source.requests = [];
      await list.check();
      await tester.pumpAndSettle();
      expect(find.text('None waiting'), findsOneWidget);
      list.stop();
    },
  );
}
