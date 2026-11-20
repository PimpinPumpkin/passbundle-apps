// Push for sign-in requests while PassBundle is closed, on Android
// (push.dart; DESIGN.md in the meta repo, "Push to phones"), against the
// real core (PB_CORE_LIB) with a stand-in for the phone's own code
// (support.dart, PushStandIn): unlocking and locking tell the phone which
// accounts are signed in and unlocked, signing out tells it which account
// goes before the account does, and Settings says whether sign-in requests
// reach PassBundle while it's closed and through which app, or how to make
// them. What the phone does with a message is Push.kt's (PushTest, and the
// emulator's phone_test.dart).
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/push.dart';

import 'support.dart';

/// An Android phone with the sample vault, unlocked with the master password.
Future<PushStandIn> unlockOnAndroid(
  WidgetTester tester,
  PushStandIn push,
) async {
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  openSample();
  forgetPushTold();
  // No biometrics here, so the vault doesn't offer them first; autofill is
  // on already, so the vault doesn't offer that either, and Settings' autofill
  // card has the phone's answer rather than waiting for one.
  PhoneStandIn(
    tester,
    status: 'unavailable',
    push: push,
    android: AndroidStandIn(autofillOn: true),
  );
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text('Logins'));
  return push;
}

/// Settings. These wait for what they read rather than for the screen to
/// settle: on Android, Settings never settled in the test's fake time, as
/// autofill_offer_test.dart's Android cases wait for their rows too.
Future<void> settings(WidgetTester tester) async {
  await tester.tap(
    find.descendant(
      of: find.byType(NavigationBar),
      matching: find.text('Settings'),
    ),
  );
  await pumpUntil(tester, find.text('Lock after'));
}

/// [finder] scrolled into view, with the scroll's frames drawn.
Future<void> inView(WidgetTester tester, Finder finder) async {
  await tester.ensureVisible(finder);
  await tester.pump(const Duration(milliseconds: 500));
}

/// Settings, scrolled to the push row: what it says.
Future<String?> pushRowSays(WidgetTester tester) async {
  await settings(tester);
  final row = find.byKey(const ValueKey('push-to-phone'));
  await pumpUntil(tester, row);
  await inView(tester, row);
  return tester.widget<Text>(
    find.descendant(of: row, matching: find.byType(Text)).last,
  ).data;
}

void main() {
  // The app's own font, as on a phone: the test font's wider letters don't
  // fit the logo on a phone's screen.
  setUpAll(loadFonts);

  testWidgets(
    'unlocking and locking tell the phone which accounts are signed in and unlocked',
    (tester) async {
      final push = await unlockOnAndroid(tester, PushStandIn());
      final account = Core.account;
      expect(account, isNotEmpty);
      expect(push.synced.last, {
        'signedIn': [account],
        'unlocked': [account],
      });

      await tester.tap(find.byTooltip('Lock'));
      await pumpUntil(tester, find.text('Unlock PassBundle'));
      expect(push.synced.last, {
        'signedIn': [account],
        'unlocked': <String>[],
      });
      // Nothing about the account is signed out.
      expect(push.forgotten, isEmpty);
    },
  );

  testWidgets('Settings says it\'s on, and through which app', (tester) async {
    await unlockOnAndroid(tester, PushStandIn(distributor: 'ntfy'));
    expect(await pushRowSays(tester), 'On, through ntfy.');
    expect(find.text('Notifications while closed'), findsOneWidget);
  });

  testWidgets(
    'with no UnifiedPush app, Settings says how sign-in requests reach PassBundle while it\'s closed',
    (tester) async {
      await unlockOnAndroid(
        tester,
        PushStandIn(installed: false, distributor: null, registered: false),
      );
      expect(await pushRowSays(tester), pushOffLine);
      expect(pushOffLine, contains('such as ntfy'));
    },
  );

  testWidgets(
    'with an app but this account not registered, Settings says it\'s off for this account',
    (tester) async {
      await unlockOnAndroid(tester, PushStandIn(registered: false));
      expect(
        await pushRowSays(tester),
        'Off for this account: its server doesn\'t send them, or ntfy '
        'hasn\'t answered yet.',
      );
    },
  );

  testWidgets(
    'signing out tells the phone which account goes, before it goes',
    (tester) async {
      final push = await unlockOnAndroid(tester, PushStandIn());
      final account = Core.account;
      await settings(tester);
      final signOut = find.byKey(const ValueKey('sign-out-account'));
      await pumpUntil(tester, signOut);
      await inView(tester, signOut);
      await tester.tap(signOut);
      await pumpUntil(tester, find.text('Sign out?'));
      // The dialog's way in, to the end, before its button is tapped.
      await tester.pump(const Duration(milliseconds: 500));
      await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
      await pumpUntil(tester, find.text('Sign in to PassBundle'));
      expect(push.forgotten, [account]);
      expect(push.synced.last, {
        'signedIn': <String>[],
        'unlocked': <String>[],
      });
    },
  );

  test('the lines say only whether, and through which app', () {
    expect(
      pushLine(const PushStatus(installed: true, distributor: 'ntfy', registered: true)),
      'On, through ntfy.',
    );
    // An app the phone can't name.
    expect(
      pushLine(const PushStatus(installed: true, registered: true)),
      'On, through your UnifiedPush app.',
    );
    expect(pushLine(const PushStatus()), pushOffLine);
  });
}
