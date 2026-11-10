// The Mac app's Touch ID, which is its own (app/macos/Runner/TouchID.swift)
// until the app is signed with an Apple developer team (the meta repo's
// docs/core.md, "The Mac without an Apple developer account"), with a
// stand-in for its Swift: the Mac's screens and words, on any computer.
// Settings offers it only where the Mac has Touch ID, as the app's own
// (not shared with the browser extension); the lock screen asks for it by
// itself as the app opens; and approving a sign-in asks for it once it's on.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/main.dart';

import 'support.dart';

/// The switch for unlocking with Touch ID, in Settings.
const biometric = ValueKey('biometric');

Future<void> unlockOnMac(WidgetTester tester) async {
  tester.view.physicalSize = const Size(1280, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  openSample();
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text('Example Mail'));
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
}

Future<void> openSettings(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Settings'));
  await tester.pump();
  await pumpUntil(tester, find.text('Lock after'));
  await pumpUntilGone(tester, find.byKey(const ValueKey('unlock-loading')));
  await tester.pump(const Duration(milliseconds: 400));
}

Finder switchedTo(bool on) => find.byWidgetPredicate(
  (w) => w is SwitchListTile && w.key == biometric && w.value == on,
);

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'the Mac offers Touch ID as its own, and turning it on and off asks the Mac',
    (tester) async {
      final mac = PhoneStandIn(tester, mac: true);
      await unlockOnMac(tester);
      await openSettings(tester);
      expect(find.text('Unlock with Touch ID'), findsOneWidget);
      expect(find.text('Instead of your master password'), findsOneWidget);
      expect(
        find.text('Also in your browser extension, which shares it'),
        findsNothing,
      );
      // Off, so approving a sign-in asks for the master password.
      expect(find.text('With your master password'), findsOneWidget);
      expect(tester.widget<SwitchListTile>(find.byKey(biometric)).value, false);

      // On: the Mac asks for Touch ID, then seals the key (its Swift).
      await tester.tap(find.byKey(biometric));
      await pumpUntil(tester, switchedTo(true));
      expect(mac.enrolled, 1);
      expect(
        find.text('With Touch ID or your master password'),
        findsOneWidget,
      );

      // Off again: it's the app's own, so nothing about the extension.
      await tester.tap(find.byKey(biometric));
      await tester.pumpAndSettle();
      expect(find.text('Turn off unlocking with Touch ID?'), findsOneWidget);
      expect(
        find.text('You can turn it on again here any time.'),
        findsOneWidget,
      );
      await tester.tap(find.text('Turn off'));
      await pumpUntil(tester, switchedTo(false));
      expect(mac.status, 'notSetUp');
    },
  );

  testWidgets(
    'a Mac where Touch ID can\'t work doesn\'t offer it, and one with no fingerprints says so',
    (tester) async {
      final mac = PhoneStandIn(tester, mac: true, status: 'unsupported');
      await unlockOnMac(tester);
      await openSettings(tester);
      expect(find.textContaining('Unlock with'), findsNothing);
      expect(find.textContaining('Touch ID'), findsNothing);
      expect(find.text('With your master password'), findsOneWidget);
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();

      // Touch ID, but no fingerprint: no switch, and why.
      mac.status = 'unavailable';
      await openSettings(tester);
      expect(find.byKey(biometric), findsNothing);
      expect(find.text('Unlock with Touch ID'), findsOneWidget);
      expect(
        find.text('Touch ID isn\'t set up on this computer'),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'signing out on the Mac says it turns Touch ID off, and nothing about the extension',
    (tester) async {
      PhoneStandIn(tester, mac: true, status: 'setUp');
      await unlockOnMac(tester);
      await openSettings(tester);
      final signOut = find.byKey(const ValueKey('settings-sign-out'));
      await tester.ensureVisible(signOut);
      await tester.pumpAndSettle();
      await tester.tap(signOut);
      await tester.pumpAndSettle();
      expect(
        find.textContaining(
          'from this Mac and turns off unlocking with Touch ID',
        ),
        findsOneWidget,
      );
      expect(find.textContaining('browser extension'), findsNothing);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
    },
  );

  testWidgets(
    'the Mac\'s lock screen asks for Touch ID by itself as the app opens, and offers it after',
    (tester) async {
      tester.view.physicalSize = const Size(1280, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final mac = PhoneStandIn(tester, mac: true, status: 'setUp');
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await pumpUntil(tester, find.text('Unlock with Touch ID'));
      for (var i = 0; i < 100 && mac.unlockPrompts < 1; i++) {
        await tester.pump(const Duration(milliseconds: 50));
      }
      await tester.pump(const Duration(milliseconds: 200));
      // Once, as it opened; the person cancelled it here.
      expect(mac.unlockPrompts, 1);
      // The button asks again; the master password still works.
      await tester.tap(find.text('Unlock with Touch ID'));
      await tester.pump(const Duration(milliseconds: 200));
      expect(mac.unlockPrompts, 2);
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Example Mail'));
    },
  );
}
