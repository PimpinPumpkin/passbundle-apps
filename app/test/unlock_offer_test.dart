// Biometric unlock, offered after the master password on a phone
// (unlock_offer.dart), against the real core (PB_CORE_LIB) with a stand-in
// for the phone's own code (support.dart, PhoneStandIn): offered once, when
// the phone has biometrics set up and unlocking with them is off; turned on
// only by the phone's prompt; not asked again for 30 days after "Not now";
// and never under an organization's "Remove Unlock with PIN" policy.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/main.dart';
import 'package:passbundle/unlock_offer.dart';

import 'support.dart';

const offer = 'Unlock with fingerprint next time?';

/// The switch for unlocking with biometrics, in Settings.
const biometric = ValueKey('biometric');

Future<void> phoneSize(WidgetTester tester) async {
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// Unlocks with the master password, and waits out what follows: the sync
/// on opening (there's no server behind the sample) and the phone's answer.
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

void main() {
  setUpAll(loadFonts);
  tearDown(() => offerClock = DateTime.now);

  testWidgets(
    'after the master password, a phone with a fingerprint offers it once, and Turn on asks for the fingerprint first',
    (tester) async {
      await phoneSize(tester);
      final phone = PhoneStandIn(tester);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      expect(find.text(offer), findsOneWidget);
      expect(phone.enrolled, 0);

      await tester.tap(find.text('Turn on'));
      await pumpUntil(tester, find.text('Unlocking with fingerprint is on.'));
      // The phone asked for the fingerprint, and only then was it on.
      expect(phone.enrolled, 1);
      expect(phone.status, 'setUp');
      await tester.pumpAndSettle();
      expect(find.text(offer), findsNothing);

      // On now, so it's not offered again.
      await lock(tester);
      expect(find.text('Unlock with fingerprint'), findsOneWidget);
      await unlockWithPassword(tester);
      expect(find.text(offer), findsNothing);
    },
  );

  testWidgets(
    'cancelling the fingerprint leaves it off, and the sheet says so',
    (tester) async {
      await phoneSize(tester);
      final phone = PhoneStandIn(tester)..biometricAnswers.add(false);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      await tester.tap(find.text('Turn on'));
      await pumpUntil(
        tester,
        find.text('It wasn\'t turned on: fingerprint was cancelled.'),
      );
      expect(phone.enrolled, 1);
      expect(phone.status, 'notSetUp');
      await tester.tap(find.text('Not now'));
      await tester.pumpAndSettle();
      expect(find.text(offer), findsNothing);
      expect(offerDeclinedAt(), isNotNull);
    },
  );

  testWidgets('"Not now" isn\'t asked again for 30 days', (tester) async {
    await phoneSize(tester);
    final phone = PhoneStandIn(tester);
    openSample();
    await tester.pumpWidget(const PassBundleApp());
    await unlockWithPassword(tester);
    await tester.tap(find.text('Not now'));
    await tester.pumpAndSettle();
    expect(find.text(offer), findsNothing);
    expect(phone.enrolled, 0);

    final declined = DateTime.now();
    for (final (days, asked) in [(0, false), (29, false), (31, true)]) {
      offerClock = () => declined.add(Duration(days: days));
      await lock(tester);
      await unlockWithPassword(tester);
      expect(
        find.text(offer),
        asked ? findsOneWidget : findsNothing,
        reason: '$days days after "Not now"',
      );
    }
    // Still nothing turns on without the fingerprint.
    expect(phone.enrolled, 0);
    expect(phone.status, 'notSetUp');
  });

  testWidgets('not when it\'s on already', (tester) async {
    await phoneSize(tester);
    PhoneStandIn(tester, status: 'setUp');
    openSample();
    await tester.pumpWidget(const PassBundleApp());
    await unlockWithPassword(tester);
    expect(find.text(offer), findsNothing);
  });

  testWidgets('not on a phone with no biometrics set up', (tester) async {
    await phoneSize(tester);
    PhoneStandIn(tester, status: 'unavailable');
    openSample();
    await tester.pumpWidget(const PassBundleApp());
    await unlockWithPassword(tester);
    expect(find.text(offer), findsNothing);
  });

  testWidgets(
    'not under an organization\'s "Remove Unlock with PIN" policy, and Settings still has the switch',
    (tester) async {
      await phoneSize(tester);
      final phone = PhoneStandIn(tester);
      openSample(
        policies: [
          {
            'id': 'p1',
            'organizationId': 'o1',
            'type': 14,
            'enabled': true,
            'data': null,
            'object': 'policy',
          },
        ],
      );
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      expect(find.text(offer), findsNothing);

      await tester.tap(find.byTooltip('Settings'));
      await pumpUntil(tester, find.byKey(biometric));
      expect(tester.widget<SwitchListTile>(find.byKey(biometric)).value, false);
      expect(phone.enrolled, 0);
    },
  );

  testWidgets(
    'turned off in Settings, it isn\'t offered again for 30 days either',
    (tester) async {
      await phoneSize(tester);
      final phone = PhoneStandIn(tester, status: 'setUp');
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      await tester.tap(find.byTooltip('Settings'));
      await pumpUntil(tester, find.byKey(biometric));
      await tester.tap(find.byKey(biometric));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Turn off'));
      await pumpUntil(
        tester,
        find.byWidgetPredicate(
          (w) => w is SwitchListTile && w.key == biometric && !w.value,
        ),
      );
      expect(phone.status, 'notSetUp');

      await tester.tap(find.byTooltip('Items'));
      await tester.pumpAndSettle();
      await lock(tester);
      await unlockWithPassword(tester);
      expect(find.text(offer), findsNothing);
    },
  );
}
