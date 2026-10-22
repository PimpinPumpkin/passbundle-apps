// The app's half of the autofill test (tools/android-autofill-test.sh): signs
// in to the test server, turns on biometric unlock, and locks, as someone's
// phone would be. Then it waits while the script fills another app's sign-in
// form from it (the test runner uninstalls the app when its tests end), until
// the script says it's done. Run only by that script.
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

Future<void> pumpUntil(WidgetTester tester, Finder finder) async {
  final end = DateTime.now().add(const Duration(seconds: 30));
  while (DateTime.now().isBefore(end)) {
    await tester.pump(const Duration(milliseconds: 100));
    if (finder.evaluate().isNotEmpty) return;
  }
  throw TestFailure('Never showed up: $finder');
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('signs in and turns on biometric unlock, for the autofill test', (
    tester,
  ) async {
    final dir = (await Core.platform.invokeMethod<String>('filesDir'))!;
    for (final kept in Directory(dir).listSync()) {
      kept.deleteSync(recursive: true);
    }
    await Core.openOnThisDevice();
    await tester.pumpWidget(PassBundleApp(key: UniqueKey()));
    await pumpUntil(tester, find.text('Sign in to PassBundle'));
    await tester.tap(find.text('Self-hosted?'));
    await tester.pump();
    await tester.enterText(find.widgetWithText(TextField, 'Server'), server);
    await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
    await tester.enterText(
      find.widgetWithText(TextField, 'Master password'),
      password,
    );
    await tester.tap(find.text('Sign in'));
    await pumpUntil(tester, find.text('Autofill test app'));

    // Debug builds' stand-in for Android's prompt says yes, here and when
    // autofill unlocks later (it's kept for the app's next process).
    expect(await Core.useBiometricStandIn(true), isTrue);
    final unlockWith =
        'Unlock with ${(await Core.biometricStatus())['method']}';
    await tester.tap(find.byTooltip('Settings'));
    await pumpUntil(tester, find.text(unlockWith));
    await tester.tap(find.text(unlockWith));
    await pumpUntil(
      tester,
      find.byWidgetPredicate(
        (w) =>
            w is SwitchListTile &&
            w.key == const ValueKey('biometric') &&
            w.value,
      ),
    );
    await tester.tap(find.text('Close'));
    await tester.pump(const Duration(milliseconds: 400));
    await tester.tap(find.byTooltip('Lock'));
    await pumpUntil(tester, find.text(unlockWith));
    expect(File('$dir/biometric.sealed').existsSync(), isTrue);

    // The script fills the other app now, then leaves this file (run-as).
    // It learns this half is ready from a file too: the test runner may hold
    // printed lines back until the test ends.
    final done = File('$dir/autofill-done');
    File('$dir/autofill-ready').writeAsStringSync('');
    final end = DateTime.now().add(const Duration(minutes: 4));
    while (!done.existsSync() && DateTime.now().isBefore(end)) {
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(seconds: 1)),
      );
    }
    expect(
      done.existsSync(),
      isTrue,
      reason: 'the script never said it was done',
    );
  }, skip: !setUp);
}
