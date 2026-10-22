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

    // The script fills the other app now. Then it signs in there with
    // something new and accepts Android's offer to save it: this app opens
    // its editor, filled in, which is saved here. It learns how far this half
    // got from files (run-as): the test runner may hold printed lines back
    // until the test ends.
    File('$dir/autofill-ready').writeAsStringSync('');
    await pumpUntil(
      tester,
      find.widgetWithText(TextField, 'new@example.test'),
      timeout: const Duration(minutes: 6),
    );
    expect(
      find.widgetWithText(TextField, 'not-a-real-password-new'),
      findsOneWidget,
    );
    expect(find.text('New item'), findsOneWidget);
    await tester.tap(find.text('Save'));
    await pumpUntil(tester, find.text('Saved'));
    // Saved for the app it was signed in to (checked in the vault: on a small
    // screen the editor's website row is below what it builds).
    final saved = Core()
        .list('')
        .where((i) => i['subtitle'] == 'new@example.test');
    expect(saved, hasLength(1));
    expect(Core().item(saved.single['id'] as String)!['uris'], [
      'androidapp://com.passbundle.autofilltest',
    ]);
    File('$dir/autofill-saved').writeAsStringSync('');

    // Then a new password for that sign-in: the same login opens, with it.
    await pumpUntil(
      tester,
      find.widgetWithText(TextField, 'not-a-real-password-changed'),
      timeout: const Duration(minutes: 6),
    );
    expect(find.text('Edit item'), findsOneWidget);
    expect(find.widgetWithText(TextField, 'new@example.test'), findsOneWidget);
    await tester.tap(find.text('Save'));
    await pumpUntil(tester, find.text('Saved the changes'));
    final logins = Core()
        .list('')
        .where((i) => i['subtitle'] == 'new@example.test')
        .toList();
    expect(logins, hasLength(1));
    expect(
      Core().reveal(logins.single['id'] as String, 'password'),
      'not-a-real-password-changed',
    );
    File('$dir/autofill-updated').writeAsStringSync('');

    // Where the script trusted the test app as a browser earlier (Android
    // 11 and later), Settings lists it, and stopping trusting it there takes
    // it off; elsewhere the script goes on to the end.
    final untrust = File('$dir/autofill-untrust');
    final done = File('$dir/autofill-done');
    var end = DateTime.now().add(const Duration(minutes: 2));
    while (!untrust.existsSync() &&
        !done.existsSync() &&
        DateTime.now().isBefore(end)) {
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(seconds: 1)),
      );
    }
    if (untrust.existsSync()) {
      // Back in front: the cover the app shows while it's away fades.
      await tester.pump(const Duration(seconds: 2));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Settings'));
      await pumpUntil(tester, find.text('Browsers you trust for autofill'));
      expect(find.text('com.passbundle.autofilltest'), findsOneWidget);
      await tester.tap(find.text('Stop trusting'));
      await pumpUntil(tester, find.text('Close'));
      for (var i = 0; i < 50; i++) {
        await tester.pump(const Duration(milliseconds: 100));
        if (find.text('Browsers you trust for autofill').evaluate().isEmpty) {
          break;
        }
      }
      expect(find.text('Browsers you trust for autofill'), findsNothing);
      await tester.tap(find.text('Close'));
      await tester.pump(const Duration(milliseconds: 400));
      File('$dir/autofill-untrusted').writeAsStringSync('');
    }

    // Then it leaves this file (run-as), and this ends.
    end = DateTime.now().add(const Duration(minutes: 2));
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
