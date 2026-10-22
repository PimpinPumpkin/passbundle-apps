// Locking by itself, against the real core (PB_CORE_LIB): the native
// runner's word that the computer is about to sleep, the core deciding on its
// own, and the choices in Settings (docs/core.md, "Locking by itself"), within
// an organization's policy ("Locking by policy"). When the core locks,
// whatever was open over the vault closes too.
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';

import 'support.dart';

Future<void> unlockSample(
  WidgetTester tester, {
  List<Map<String, dynamic>>? policies,
}) async {
  tester.view.physicalSize = const Size(1280, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  // Each test turns "when the computer sleeps" on itself, right where it's about it.
  openSample(policies: policies);
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text('Backup service'));
  // The sync on opening is done (there's no server), so the core is free.
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
}

Future<void> openSettings(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Settings'));
  await tester.pump();
  // Its first line waits on the desktop agent (a real call, outside the test's clock).
  await pumpUntil(tester, find.textContaining('Unlock with'));
  await tester.pump(const Duration(milliseconds: 400));
  expect(find.text('Lock after'), findsOneWidget);
}

/// What the runner sends when the OS says the computer is about to sleep.
Future<void> computerSleeps(WidgetTester tester) async {
  await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
    'passbundle/computer',
    const StandardMethodCodec().encodeMethodCall(const MethodCall('sleeping')),
    (_) {},
  );
  await tester.pump();
}

void main() {
  setUpAll(loadFonts);

  testWidgets('the computer going to sleep locks it and closes what was open', (
    tester,
  ) async {
    await unlockSample(tester);
    await openSettings(tester);
    Core().setPreferences({'lockAfterMinutes': 15, 'lockOnSleep': true});
    await computerSleeps(tester);
    expect(find.text('Unlock PassBundle'), findsOneWidget);
    expect(find.text('Lock after'), findsNothing);
    expect(find.text('Backup service'), findsNothing);
    expect(Core().state()['state'], 'locked');
  });

  testWidgets(
    'when the core locks by itself, the app shows it within two seconds',
    (tester) async {
      await unlockSample(tester);
      await openSettings(tester);
      // As when the time runs out or the screen locks: the core decides.
      Core().lock();
      await tester.pump(const Duration(seconds: 2));
      await tester.pump();
      expect(find.text('Unlock PassBundle'), findsOneWidget);
      expect(find.text('Lock after'), findsNothing);
    },
  );

  testWidgets('Settings chooses when it locks, and the core keeps the choice', (
    tester,
  ) async {
    await unlockSample(tester);
    await openSettings(tester);
    expect(find.text('15 minutes'), findsOneWidget);

    await tester.tap(find.text('15 minutes'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('1 hour').last);
    await tester.pumpAndSettle();
    expect(Core().preferences()['lockAfterMinutes'], 60);

    await tester.tap(find.text('1 hour'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('When the app closes').last);
    await tester.pumpAndSettle();
    expect(Core().preferences()['lockAfterMinutes'], isNull);

    // On, then off again at once: no time passes for the app's check, which
    // would lock on a test machine whose screen is locked.
    await tester.tap(find.text('Lock when the computer sleeps'));
    await tester.pump();
    expect(Core().preferences()['lockOnSleep'], isTrue);
    await tester.tap(find.text('Lock when the computer sleeps'));
    await tester.pumpAndSettle();
    expect(Core().preferences()['lockOnSleep'], isFalse);

    // Chosen off, sleeping leaves it open.
    await tester.tap(find.text('Close'));
    await tester.pumpAndSettle();
    await computerSleeps(tester);
    expect(find.text('Backup service'), findsWidgets);
    expect(Core().state()['state'], 'unlocked');
  });

  /// An organization's "Vault lock" policy, as the server sends it with a sync.
  Map<String, dynamic> vaultLock(Map<String, dynamic> data) => {
    'id': 'p1',
    'organizationId': 'o1',
    'type': 9,
    'enabled': true,
    'data': data,
    'object': 'policy',
  };

  testWidgets(
    "Settings offers only the waits an organization's policy allows, and says why",
    (tester) async {
      await unlockSample(
        tester,
        policies: [
          vaultLock({'type': 'custom', 'minutes': 30, 'action': 'logOut'}),
        ],
      );
      await openSettings(tester);
      // The person's 15 minutes is within it, and stays.
      expect(find.text('15 minutes'), findsOneWidget);
      expect(
        find.textContaining(
          'Your organization signs you out after 30 minutes at most',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('15 minutes'));
      await tester.pump(const Duration(milliseconds: 400));
      for (final allowed in ['1 minute', '5 minutes', '30 minutes']) {
        expect(find.text(allowed), findsWidgets);
      }
      expect(find.text('1 hour'), findsNothing);
      expect(find.text('When the app closes'), findsNothing);
      await tester.tap(find.text('30 minutes').last);
      await tester.pump(const Duration(milliseconds: 400));
      expect(Core().preferences()['lockAfterMinutes'], 30);
    },
  );

  testWidgets(
    "a longer wait chosen before the policy shows as the policy's, and stays the person's",
    (tester) async {
      await unlockSample(
        tester,
        policies: [
          vaultLock({'minutes': 5}),
        ],
      );
      Core().setPreferences({'lockAfterMinutes': null, 'lockOnSleep': false});
      await openSettings(tester);
      expect(find.text('5 minutes'), findsOneWidget);
      expect(
        find.textContaining(
          'Your organization locks PassBundle after 5 minutes at most',
        ),
        findsOneWidget,
      );
      expect(Core().preferences()['lockAfterMinutes'], isNull);
    },
  );

  testWidgets(
    "with the policy's \"when the computer locks\", that switch stays on",
    (tester) async {
      await unlockSample(
        tester,
        policies: [
          vaultLock({'type': 'onSystemLock'}),
        ],
      );
      await openSettings(tester);
      final sleep = tester.widget<SwitchListTile>(
        find.widgetWithText(SwitchListTile, 'Lock when the computer sleeps'),
      );
      expect(sleep.value, isTrue);
      expect(sleep.onChanged, isNull);
      expect(
        find.textContaining('Your organization keeps this on'),
        findsOneWidget,
      );
      // The person's own choice is still theirs.
      expect(Core().preferences()['lockOnSleep'], isFalse);
    },
  );
}
