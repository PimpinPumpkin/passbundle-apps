// The app's half of the iOS AutoFill test (tools/ios-autofill-test.sh): signs
// in to the test server and turns on Face ID unlock (debug builds' stand-in
// for the prompt), as someone's iPhone would be, then waits while the script
// turns PassBundle on for AutoFill, gives iOS its suggestions when told, and
// waits while Safari fills from them (the test runner uninstalls the app when
// its tests end). The two halves talk through files in the App Group folder,
// which the script reaches from the Mac. Run only by that script.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/widgets.dart';

const server = String.fromEnvironment('PB_TEST_SERVER');
const email = String.fromEnvironment('PB_TEST_EMAIL');
const password = String.fromEnvironment('PB_TEST_PASSWORD');
const setUp = bool.fromEnvironment('PB_AUTOFILL_SETUP');

Future<void> pumpUntil(
  WidgetTester tester,
  Finder finder, {
  Duration timeout = const Duration(seconds: 30),
}) async {
  final end = DateTime.now().add(timeout);
  while (DateTime.now().isBefore(end)) {
    await tester.pump(const Duration(milliseconds: 100));
    if (finder.evaluate().isNotEmpty) return;
  }
  throw TestFailure('Never showed up: $finder');
}

/// Waits for the script to leave a file in the App Group folder.
Future<void> waitFor(WidgetTester tester, File file, Duration timeout) async {
  final end = DateTime.now().add(timeout);
  while (!file.existsSync() && DateTime.now().isBefore(end)) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(seconds: 1)),
    );
  }
  expect(file.existsSync(), isTrue, reason: 'the script never left $file');
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets(
    'signs in, turns on Face ID unlock, and gives iOS its suggestions, for the AutoFill test',
    (tester) async {
      final dir = (await Core.platform.invokeMethod<String>('filesDir'))!;
      for (final kept in Directory(dir).listSync()) {
        kept.deleteSync(recursive: true);
      }
      // The Keychain keeps items across installs on the simulator: a Face ID
      // item left by an earlier run would stand in the way.
      await Core.removeBiometric();
      await Core.openOnThisDevice();
      await tester.pumpWidget(PassBundleApp(key: UniqueKey()));
      await pumpUntil(tester, find.text('Sign in to PassBundle'));
      if (find.text('Self-hosted?').evaluate().isNotEmpty) {
        await tester.tap(find.text('Self-hosted?'));
        await tester.pump();
      }
      await tester.enterText(find.widgetWithText(TextField, 'Server'), server);
      await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        password,
      );
      await tester.tap(find.text('Sign in'));
      await pumpUntil(tester, find.text('Saved before the phone signed in'));

      // Face ID on, with debug builds' stand-in, so the extension unlocks
      // with the Keychain item without a prompt.
      expect(await Core.useBiometricStandIn(true), isTrue);
      // The sync that follows signing in has the core until it's done, and
      // turning on Face ID needs it (the user key): wait for it.
      for (var i = 0; i < 300 && busy.value; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      final enrolled = await Core.enrollBiometric();
      expect(enrolled.ok, isTrue, reason: enrolled.error);
      File('$dir/autofill-ready').writeAsStringSync('');

      // Once the script has turned PassBundle on for AutoFill, iOS takes the
      // suggestions: the logins with a website (the test account's other
      // tests add more).
      await waitFor(
        tester,
        File('$dir/autofill-enabled'),
        const Duration(minutes: 4),
      );
      // iOS may take a moment to say AutoFill is on after the switch moves.
      var suggested = await Core.suggest();
      for (var i = 0; i < 20 && suggested['stored'] != true; i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 500)),
        );
        suggested = await Core.suggest();
      }
      expect(suggested['stored'], isTrue);
      expect(suggested['count'], greaterThanOrEqualTo(1));
      File('$dir/autofill-suggested').writeAsStringSync('');

      // Safari fills from them now, and makes a passkey on the site: the
      // script says when, and the login it went in is in PassBundle's vault
      // (a passkey saved anywhere else, such as Apple's Passwords, isn't).
      final passkeys = File('$dir/autofill-passkeys');
      final done = File('$dir/autofill-done');
      final end = DateTime.now().add(const Duration(minutes: 15));
      while (!passkeys.existsSync() &&
          !done.existsSync() &&
          DateTime.now().isBefore(end)) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(seconds: 1)),
        );
      }
      if (passkeys.existsSync()) {
        final synced = await Core.sync();
        expect(synced.ok, isTrue, reason: synced.error);
        final made = Core()
            .list('phone.example.test')
            .where((i) => i['name'] == 'phone.example.test');
        expect(made, isNotEmpty, reason: 'the passkey isn\'t in PassBundle');
        File('$dir/autofill-passkey-saved').writeAsStringSync('');
      }

      // Then the script says it's done.
      await waitFor(tester, done, const Duration(minutes: 6));
    },
    skip: !setUp,
  );
}
