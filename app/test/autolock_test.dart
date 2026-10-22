// Locking by itself, against the real core (PB_CORE_LIB): the native
// runner's word that the computer is about to sleep, the core deciding on its
// own, and the choices in Settings (docs/core.md, "Locking by itself"). When
// the core locks, whatever was open over the vault closes too.
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';

import 'support.dart';

Future<void> unlockSample(WidgetTester tester) async {
  tester.view.physicalSize = const Size(1280, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  // Each test turns "when the computer sleeps" on itself, right where it's about it.
  openSample();
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
}
