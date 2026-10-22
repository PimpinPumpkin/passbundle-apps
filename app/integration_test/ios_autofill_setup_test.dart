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
      expect((await Core.enrollBiometric()).ok, isTrue);
      File('$dir/autofill-ready').writeAsStringSync('');

      // Once the script has turned PassBundle on for AutoFill, iOS takes the
      // suggestions: the one login with a website.
      await waitFor(
        tester,
        File('$dir/autofill-enabled'),
        const Duration(minutes: 4),
      );
      expect(await Core.suggest(), {'count': 1, 'stored': true});
      File('$dir/autofill-suggested').writeAsStringSync('');

      // Safari fills from them now; then the script says it's done.
      await waitFor(
        tester,
        File('$dir/autofill-done'),
        const Duration(minutes: 6),
      );
    },
    skip: !setUp,
  );
}
