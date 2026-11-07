// Turning on autofill on Android (autofill_offer.dart): what the card shows
// from what Android says, and the step as the vault opens, against the real
// core (PB_CORE_LIB) with a stand-in for the phone's own code (support.dart,
// PhoneStandIn and AndroidStandIn). The step offers autofill until it's on or
// the person says "Later"; "Turn on" opens Android's own prompt, and a row
// says On once Android says so, when the app is back in front; Android 14 and
// later have a row for passkeys too; Settings has the same card. Nowhere but
// Android.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/autofill_offer.dart';
import 'package:passbundle/main.dart';

import 'support.dart';

const step = 'Turn on autofill';
const autofillRow = ValueKey('setup-autofill');
const passkeysRow = ValueKey('setup-passkeys');
const turnOnAutofill = ValueKey('turn-on-autofill');
const turnOnPasskeys = ValueKey('turn-on-passkeys');

Future<void> phoneSize(WidgetTester tester) async {
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// Unlocks with the master password, and waits out what follows: the sync
/// on opening (there's no server behind the sample) and the phone's answers.
Future<void> unlockWithPassword(WidgetTester tester) async {
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text('Logins'));
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
  await tester.pumpAndSettle();
}

Future<void> lock(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Lock'));
  await tester.pumpAndSettle();
  expect(find.text('Unlock PassBundle'), findsOneWidget);
}

/// Back from Android's prompt or settings: the app in front again, which
/// asks Android again.
Future<void> comeBack(WidgetTester tester) async {
  tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
  tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
  await tester.pump();
  await tester.pump(const Duration(seconds: 1));
}

/// A row says On, with a check.
Finder isOn(Key row) => find.descendant(
  of: find.byKey(row),
  matching: find.byIcon(Icons.check_circle_rounded),
);

void main() {
  setUpAll(loadFonts);

  group('what the card shows', () {
    test('Android 10 to 13: autofill alone, even with Credential Manager', () {
      for (var api = 29; api <= 33; api++) {
        expect(
          setupRows(AutofillStatus(apiLevel: api, passkeysAvailable: true)),
          [(SetupRow.autofill, false)],
          reason: 'API $api',
        );
      }
    });

    test('Android 14 and later: passkeys too, where Credential Manager is', () {
      for (final api in [34, 35, 36]) {
        expect(
          setupRows(AutofillStatus(apiLevel: api, passkeysAvailable: true)),
          [(SetupRow.autofill, false), (SetupRow.passkeys, false)],
          reason: 'API $api',
        );
        expect(setupRows(AutofillStatus(apiLevel: api)), [
          (SetupRow.autofill, false),
        ], reason: 'API $api without Credential Manager');
      }
    });

    test('each row says whether it\'s on', () {
      expect(
        setupRows(
          const AutofillStatus(
            apiLevel: 34,
            autofillOn: true,
            passkeysAvailable: true,
          ),
        ),
        [(SetupRow.autofill, true), (SetupRow.passkeys, false)],
      );
      expect(
        setupRows(
          const AutofillStatus(
            apiLevel: 35,
            passkeysAvailable: true,
            passkeysOn: true,
          ),
        ),
        [(SetupRow.autofill, false), (SetupRow.passkeys, true)],
      );
    });

    test('a phone without autofill has only passkeys, or nothing', () {
      expect(
        setupRows(
          const AutofillStatus(
            apiLevel: 33,
            autofillSupported: false,
            passkeysAvailable: true,
          ),
        ),
        isEmpty,
      );
      expect(
        setupRows(
          const AutofillStatus(
            apiLevel: 34,
            autofillSupported: false,
            passkeysAvailable: true,
          ),
        ),
        [(SetupRow.passkeys, false)],
      );
    });

    test('the step is offered only while autofill is off, until "Later"', () {
      const off = AutofillStatus(apiLevel: 34, passkeysAvailable: true);
      expect(offersAutofill(off, putOff: false), isTrue);
      expect(offersAutofill(off, putOff: true), isFalse);
      // Passkeys off don't bring it back: Settings has them.
      expect(
        offersAutofill(
          const AutofillStatus(
            apiLevel: 34,
            autofillOn: true,
            passkeysAvailable: true,
          ),
          putOff: false,
        ),
        isFalse,
      );
      expect(
        offersAutofill(
          const AutofillStatus(apiLevel: 29, autofillSupported: false),
          putOff: false,
        ),
        isFalse,
      );
      // Not Android.
      expect(offersAutofill(null, putOff: false), isFalse);
    });
  });

  testWidgets(
    'as the vault opens, the step offers autofill; Turn on opens Android\'s prompt, and once Android says it\'s on, the step closes',
    (tester) async {
      await phoneSize(tester);
      final android = AndroidStandIn(apiLevel: 33);
      PhoneStandIn(tester, status: 'unavailable', android: android);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      expect(find.text(step), findsOneWidget);
      expect(find.text('Use PassBundle for autofill'), findsOneWidget);
      // Android 13: no passkeys row.
      expect(find.byKey(passkeysRow), findsNothing);
      expect(find.text('Later'), findsOneWidget);

      await tester.tap(find.byKey(turnOnAutofill));
      await tester.pump();
      expect(android.opened, ['autofill']);
      // Android's prompt is over the app: nothing's known until it's back.
      expect(find.text(step), findsOneWidget);
      expect(isOn(autofillRow), findsNothing);

      await comeBack(tester);
      await pumpUntil(tester, find.text('Autofill is on.'));
      await tester.pumpAndSettle();
      expect(find.text(step), findsNothing);
      expect(autofillPutOff(), isFalse);

      // On now, so it's not offered again.
      await lock(tester);
      await unlockWithPassword(tester);
      expect(find.text(step), findsNothing);
    },
  );

  testWidgets(
    'Android 14: a row for passkeys opens the system\'s passkey settings, and the step closes once both are on',
    (tester) async {
      await phoneSize(tester);
      final android = AndroidStandIn(apiLevel: 34);
      PhoneStandIn(tester, status: 'unavailable', android: android);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      expect(find.text(step), findsOneWidget);
      expect(find.text('Use PassBundle for passkeys'), findsOneWidget);

      await tester.tap(find.byKey(turnOnAutofill));
      await tester.pump();
      await comeBack(tester);
      await tester.pumpAndSettle();
      // Autofill is on; passkeys are left, so the step stays, with Done.
      expect(isOn(autofillRow), findsOneWidget);
      expect(find.byKey(turnOnAutofill), findsNothing);
      expect(isOn(passkeysRow), findsNothing);
      expect(find.text('Done'), findsOneWidget);

      await tester.tap(find.byKey(turnOnPasskeys));
      await tester.pump();
      expect(android.opened, ['autofill', 'passkeys']);
      await comeBack(tester);
      await pumpUntil(tester, find.text('Autofill and passkeys are on.'));
      await tester.pumpAndSettle();
      expect(find.text(step), findsNothing);
    },
  );

  testWidgets(
    'after the biometric offer, then "Later" isn\'t asked again, and Settings has the same card',
    (tester) async {
      await phoneSize(tester);
      final android = AndroidStandIn(apiLevel: 34);
      PhoneStandIn(tester, android: android);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      // Biometric unlock is offered first.
      expect(find.text('Unlock with fingerprint next time?'), findsOneWidget);
      expect(find.text(step), findsNothing);
      await tester.tap(find.text('Not now'));
      await pumpUntil(tester, find.text(step));
      await tester.pumpAndSettle();

      await tester.tap(find.text('Later'));
      await tester.pumpAndSettle();
      expect(find.text(step), findsNothing);
      expect(autofillPutOff(), isTrue);
      // Where it is from now on, a tap away.
      expect(
        find.text('You can turn on autofill in Settings any time.'),
        findsOneWidget,
      );
      expect(find.widgetWithText(SnackBarAction, 'Settings'), findsOneWidget);
      expect(android.opened, isEmpty);
      expect(android.autofillOn, isFalse);

      await tester.tap(find.byTooltip('Settings'));
      await pumpUntil(tester, find.byKey(autofillRow));
      expect(find.text('Autofill'), findsOneWidget);
      expect(find.byKey(passkeysRow), findsOneWidget);
      await tester.tap(find.byKey(turnOnAutofill));
      await tester.pump();
      expect(android.opened, ['autofill']);
      await comeBack(tester);
      await tester.pumpAndSettle();
      expect(isOn(autofillRow), findsOneWidget);
      expect(find.byKey(turnOnPasskeys), findsOneWidget);

      await tester.tap(find.byTooltip('Items'));
      await tester.pumpAndSettle();
      await lock(tester);
      await unlockWithPassword(tester);
      expect(find.text(step), findsNothing);
    },
  );

  testWidgets(
    '"No" in Android\'s prompt leaves it off, and the step stays',
    (tester) async {
      await phoneSize(tester);
      final android = AndroidStandIn(apiLevel: 30)..yes = false;
      PhoneStandIn(tester, status: 'unavailable', android: android);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      await tester.tap(find.byKey(turnOnAutofill));
      await tester.pump();
      await comeBack(tester);
      await tester.pumpAndSettle();
      expect(find.text(step), findsOneWidget);
      expect(find.byKey(turnOnAutofill), findsOneWidget);
      expect(find.text('Later'), findsOneWidget);
    },
  );

  testWidgets(
    'where Android has no prompt and nothing opens, the step says where to look',
    (tester) async {
      await phoneSize(tester);
      final android = AndroidStandIn(apiLevel: 31)..opens = false;
      PhoneStandIn(tester, status: 'unavailable', android: android);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      await tester.tap(find.byKey(turnOnAutofill));
      await tester.pumpAndSettle();
      expect(find.textContaining('search for Autofill service'), findsOneWidget);
    },
  );

  testWidgets('not when autofill is on already; Settings says On', (
    tester,
  ) async {
    await phoneSize(tester);
    PhoneStandIn(
      tester,
      status: 'unavailable',
      android: AndroidStandIn(apiLevel: 29, autofillOn: true),
    );
    openSample();
    await tester.pumpWidget(const PassBundleApp());
    await unlockWithPassword(tester);
    expect(find.text(step), findsNothing);
    await tester.tap(find.byTooltip('Settings'));
    await pumpUntil(tester, find.byKey(autofillRow));
    expect(isOn(autofillRow), findsOneWidget);
    expect(find.byKey(turnOnAutofill), findsNothing);
  });

  testWidgets('not where Android has no autofill, and Settings has no card', (
    tester,
  ) async {
    await phoneSize(tester);
    PhoneStandIn(
      tester,
      status: 'unavailable',
      android: AndroidStandIn(apiLevel: 33, autofillSupported: false),
    );
    openSample();
    await tester.pumpWidget(const PassBundleApp());
    await unlockWithPassword(tester);
    expect(find.text(step), findsNothing);
    await tester.tap(find.byTooltip('Settings'));
    await pumpUntil(tester, find.text('Lock after'));
    await tester.pumpAndSettle();
    expect(find.byKey(autofillRow), findsNothing);
    expect(find.text('Autofill'), findsNothing);
  });

  testWidgets('nowhere but Android: an iPhone has neither', (tester) async {
    await phoneSize(tester);
    PhoneStandIn(tester, status: 'unavailable');
    openSample();
    await tester.pumpWidget(const PassBundleApp());
    await unlockWithPassword(tester);
    expect(find.text(step), findsNothing);
    await tester.tap(find.byTooltip('Settings'));
    await pumpUntil(tester, find.text('Lock after'));
    await tester.pumpAndSettle();
    expect(find.text('Autofill'), findsNothing);
  });
}
