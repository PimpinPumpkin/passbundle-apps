// The app on a real phone, an Android emulator, or an iPhone simulator
// (docs/phones.md, "Order and tests"), with the real core: it opens the vault
// kept in the app's own folder, unlocks it, copies a password straight from
// the core to the clipboard through the app's Kotlin or Swift, locks, and
// checks certificates with the OS's own verifier. Run it with
// tools/android-app-test.sh or tools/ios-app-test.sh, which make
// fixtures.g.dart (the sample vault and a throwaway self-signed certificate).
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';

import 'fixtures.g.dart';

/// Pumps until something shows up, giving slow core calls real time.
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
  final shown = find
      .byType(Text)
      .evaluate()
      .map((e) => (e.widget as Text).data)
      .whereType<String>()
      .join(' | ');
  throw TestFailure('Never showed up: $finder\nOn screen: $shown');
}

/// Taps something once the screen has stopped moving under it: it can be
/// hit, and it's in the same place for a few frames (a keyboard closing
/// moves the screen after a field is typed in, and a tap mid-move misses).
Future<void> tapWhenStill(WidgetTester tester, Finder finder) async {
  Offset? last;
  var still = 0;
  final end = DateTime.now().add(const Duration(seconds: 30));
  while (still < 3 && DateTime.now().isBefore(end)) {
    await tester.pump(const Duration(milliseconds: 100));
    final found = finder.hitTestable().evaluate();
    final at = found.isEmpty ? null : tester.getCenter(finder);
    still = at != null && at == last ? still + 1 : 0;
    last = at;
  }
  if (still < 3) throw TestFailure('Never held still: $finder');
  await tester.tap(finder);
}

// A real server the phone reaches (tools/android-app-test.sh, with
// PB_TEST_SERVER), and an account on it with one login.
const server = String.fromEnvironment('PB_TEST_SERVER');
const email = String.fromEnvironment('PB_TEST_EMAIL');
const password = String.fromEnvironment('PB_TEST_PASSWORD');

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets(
    'unlocks the kept vault, copies straight from the core, locks, and refuses an untrusted server',
    (tester) async {
      // The sample vault, kept in the app's own folder as if signed in.
      final dir = await Core.platform.invokeMethod<String>('filesDir');
      File('$dir/settings.json').writeAsStringSync(sampleSettings);
      File('$dir/sync.json').writeAsStringSync(sampleSync);
      await Core.openOnThisDevice();
      await tester.pumpWidget(const PassBundleApp());
      await pumpUntil(tester, find.text('Unlock PassBundle'));

      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Backup service'));
      // No refresh token is kept yet on Android, so syncing asks to sign in again.
      await pumpUntil(tester, find.textContaining('Sign in again'));

      await tester.tap(find.text('Backup service'));
      // The item's own page, past its slide-in.
      await pumpUntil(tester, find.byTooltip('Copy password'));
      await tester.pump(const Duration(milliseconds: 500));
      await tester.tap(find.byTooltip('Copy password'));
      await pumpUntil(
        tester,
        find.text('Copied the password. It clears in 30 seconds.'),
      );
      // Read back here only to check it; the app itself never has it in Dart.
      final copied = await Clipboard.getData(Clipboard.kTextPlain);
      expect(copied?.text, samplePasswordOfBackupService);
      final copiedAt = DateTime.now();

      // Back to the list, where the header has Lock.
      await tester.pageBack();
      // Past the way back (iOS's is slower), where Lock can be tapped.
      await pumpUntil(tester, find.byTooltip('Lock').hitTestable());
      await tester.tap(find.byTooltip('Lock'));
      await pumpUntil(tester, find.text('Unlock PassBundle'));
      expect(find.text('Backup service'), findsNothing);

      // Android's own verifier checks the server's certificate: a self-signed
      // one is refused, rather than the core stopping for want of its setup.
      final context = SecurityContext()
        ..useCertificateChainBytes(certificatePem.codeUnits)
        ..usePrivateKeyBytes(privateKeyPem.codeUnits);
      final server = await HttpServer.bindSecure('localhost', 0, context);
      server.listen((request) => request.response.close());
      try {
        await tester.tap(find.text('Sign out').first);
        await pumpUntil(tester, find.text('Sign out?'));
        await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
        await pumpUntil(tester, find.text('Sign in to PassBundle'));
        // The sample vault's server isn't PassBundle's, so its field shows.
        await tester.enterText(
          find.widgetWithText(TextField, 'Server'),
          'https://localhost:${server.port}',
        );
        await tester.enterText(
          find.widgetWithText(TextField, 'Email'),
          'ana@example.test',
        );
        await tester.enterText(
          find.widgetWithText(TextField, 'Master password'),
          'anything',
        );
        // The keyboard away, and past the sign-out dialog's way out, where
        // Sign in can be tapped.
        FocusManager.instance.primaryFocus?.unfocus();
        await tapWhenStill(tester, find.text('Sign in'));
        await pumpUntil(tester, find.textContaining('certificate'));
        expect(
          find.textContaining('Couldn\'t reach the server'),
          findsOneWidget,
        );
      } finally {
        await server.close(force: true);
      }

      // 30 seconds after copying, the password is off the clipboard.
      final waited = DateTime.now().difference(copiedAt);
      await Future<void>.delayed(const Duration(seconds: 32) - waited);
      final later = await Clipboard.getData(Clipboard.kTextPlain);
      expect(later?.text ?? '', isEmpty);
    },
  );

  testWidgets(
    'signs in to a real server, and the Keystore or Keychain keeps the session across a restart',
    (tester) async {
      // From nothing kept on the phone.
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
      FocusManager.instance.primaryFocus?.unfocus();
      await tapWhenStill(tester, find.text('Sign in'));
      await pumpUntil(tester, find.text('Saved before the phone signed in'));
      if (Platform.isIOS) {
        // Kept where the AutoFill extension reads it too, and iOS may suggest
        // the logins with a website, by their usernames and domains (the
        // test account's other tests add more).
        expect(dir, contains('/Shared/AppGroup/'));
        expect((await Core.suggest())['count'], greaterThanOrEqualTo(1));
      }

      if (Platform.isAndroid) {
        // Kept sealed by the Keystore: one file, of the IV, ciphertext, and
        // tag, not the token (which is printable base64) as text.
        final tokens = Directory('$dir/tokens').listSync();
        expect(tokens, hasLength(1));
        final sealed = (tokens.single as File).readAsBytesSync();
        expect(sealed.length, greaterThan(12 + 16));
        expect(sealed.any((b) => b < 0x20 || b > 0x7e), isTrue);
      } else {
        // In the Keychain, never in the app's folder.
        expect(Directory('$dir/tokens').existsSync(), isFalse);
      }

      // Its attachment saves to the phone's own Downloads (Android) or Files
      // (iPhone): the core downloads it, opens it, and writes it there itself.
      await tester.tap(find.text('Saved before the phone signed in'));
      // Past the way in (iOS's slides from the right), where Save can be tapped.
      await pumpUntil(tester, find.byTooltip('Save notes.txt').hitTestable());
      await tester.tap(find.byTooltip('Save notes.txt'));
      await pumpUntil(
        tester,
        find.textContaining(
          Platform.isAndroid
              ? 'Saved to Downloads/notes'
              : 'Saved to Files, On My iPhone, PassBundle, notes',
        ),
      );

      // A restart: the core opens again from only what's kept. The vault
      // unlocks, and it syncs with the kept session, without signing in.
      await Core.openOnThisDevice();
      await tester.pumpWidget(PassBundleApp(key: UniqueKey()));
      await pumpUntil(tester, find.text('Unlock PassBundle'));
      await tester.enterText(find.byType(TextField), password);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Saved before the phone signed in'));
      await pumpUntil(tester, find.byTooltip('Sync'));
      expect(find.textContaining('Sign in again'), findsNothing);
    },
    skip: server.isEmpty,
  );

  testWidgets(
    'turns on biometric unlock, unlocks with it, and signing out turns it off',
    (tester) async {
      // Debug builds' stand-in for the phone's prompt says yes; the real
      // prompt is checked by hand on a real phone (docs/phones.md).
      expect(await Core.useBiometricStandIn(true), isTrue);
      addTearDown(() => Core.useBiometricStandIn(false));
      final unlockWith =
          'Unlock with ${(await Core.biometricStatus())['method']}';
      final dir = (await Core.platform.invokeMethod<String>('filesDir'))!;
      for (final kept in Directory(dir).listSync()) {
        kept.deleteSync(recursive: true);
      }
      File('$dir/settings.json').writeAsStringSync(sampleSettings);
      File('$dir/sync.json').writeAsStringSync(sampleSync);
      await Core.removeBiometric();
      await Core.openOnThisDevice();
      await tester.pumpWidget(PassBundleApp(key: UniqueKey()));
      await pumpUntil(tester, find.text('Unlock PassBundle'));
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Backup service'));

      // Turning it on: Android asks (the stand-in, here), then the user key is sealed.
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
      // Sealed by the Keystore in the app's folder, or in the Keychain.
      if (Platform.isAndroid) {
        expect(File('$dir/biometric.sealed').existsSync(), isTrue);
      }
      expect((await Core.biometricStatus())['status'], 'setUp');
      await tester.tap(find.text('Close'));
      await tester.pump(const Duration(milliseconds: 400));

      // Unlocking with it: no master password.
      await tester.tap(find.byTooltip('Lock'));
      await pumpUntil(tester, find.text(unlockWith));
      await tester.tap(find.text(unlockWith));
      await pumpUntil(tester, find.text('Backup service'));

      // Signing out turns it off: the core asks Kotlin to delete the key and the sealed user key.
      await tester.tap(find.byTooltip('Lock'));
      await pumpUntil(tester, find.text('Sign out'));
      await tester.tap(find.text('Sign out').first);
      await pumpUntil(tester, find.text('Sign out?'));
      await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
      await pumpUntil(tester, find.text('Sign in to PassBundle'));
      if (Platform.isAndroid) {
        expect(File('$dir/biometric.sealed').existsSync(), isFalse);
      }
      expect((await Core.biometricStatus())['status'], 'notSetUp');
    },
  );

  testWidgets('chosen "Immediately", leaving the app locks it', (tester) async {
    final dir = (await Core.platform.invokeMethod<String>('filesDir'))!;
    for (final kept in Directory(dir).listSync()) {
      kept.deleteSync(recursive: true);
    }
    File('$dir/settings.json').writeAsStringSync(sampleSettings);
    File('$dir/sync.json').writeAsStringSync(sampleSync);
    await Core.openOnThisDevice();
    await tester.pumpWidget(PassBundleApp(key: UniqueKey()));
    await pumpUntil(tester, find.text('Unlock PassBundle'));
    await tester.enterText(find.byType(TextField), samplePassword);
    await tester.tap(find.text('Unlock'));
    await pumpUntil(tester, find.text('Backup service'));

    // A phone's own words and choices: 5 minutes away by default.
    await tester.tap(find.byTooltip('Settings'));
    await pumpUntil(tester, find.text('Lock when the phone locks'));
    expect(find.text('Away from PassBundle'), findsOneWidget);
    // Settings scrolls on a small screen, as CI's emulator has.
    await tester.ensureVisible(find.text('5 minutes'));
    await tester.pump(const Duration(milliseconds: 300));
    await tester.tap(find.text('5 minutes'));
    await pumpUntil(tester, find.text('Immediately'));
    await tester.tap(find.text('Immediately').last);
    await tester.pump(const Duration(milliseconds: 500));
    expect(Core().preferences()['lockAfterMinutes'], 0);
    await tester.tap(find.text('Close'));
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Backup service'), findsWidgets);

    // Home, or the app switcher: the app leaves the screen. It locks right
    // then (no frame comes while it's hidden, so nothing is pumped here).
    final binding = tester.binding;
    binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    expect(Core().state()['state'], 'locked');
    binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await pumpUntil(tester, find.text('Unlock PassBundle'));
  });
}
