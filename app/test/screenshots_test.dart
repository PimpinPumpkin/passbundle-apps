// Screenshots for people to look at, not checks: run with --update-goldens
// to (re)draw them in test/screenshots/. Desktop and phone width, light and
// dark; the phone's at 390 wide, and Settings also at 360 with the text 1.3
// times its size, as a phone set to large text draws it. With PB_SHOWCASE
// (core/examples/showcase.rs), on a fuller vault: folders in folders,
// shared folders, and items made from templates.
@Tags(['screenshots'])
library;

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/updates.dart';

import 'support.dart';

Future<void> size(WidgetTester tester, Size size, {double text = 1}) async {
  tester.view.physicalSize = size * 2;
  tester.view.devicePixelRatio = 2;
  tester.platformDispatcher.textScaleFactorTestValue = text;
  addTearDown(tester.view.reset);
  addTearDown(tester.platformDispatcher.clearAllTestValues);
}

Future<void> shoot(String name) => expectLater(
  find.byType(MaterialApp),
  matchesGoldenFile('screenshots/$name.png'),
);

Future<void> unlock(WidgetTester tester, {bool phone = false}) async {
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  // A phone's vault starts with its kinds; a computer's lists the items.
  await pumpUntil(tester, find.text(phone ? 'Logins' : 'Backup service'));
  // The offline banner (no server behind the sample) settles first.
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
}

/// Android's updates, as Settings shows them there: this version, checked.
class _Updates implements UpdatePlatform {
  @override
  Future<Map<String, dynamic>> status() async => {
    'version': '0.1.4',
    'store': null,
    'auto': true,
  };

  @override
  Future<Map<String, dynamic>> check({required bool quiet}) async => {
    'ok': true,
    'available': false,
  };

  @override
  Future<Map<String, dynamic>> install() async => {'ok': true};

  @override
  Future<void> setAuto(bool on) async {}

  @override
  set onEvent(void Function(String, Map<String, dynamic>) handler) {}

  // Anything else the native code does for updates, which these don't show.
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

/// The switch for unlocking with Windows Hello or Touch ID, in Settings.
const biometric = ValueKey('biometric');

/// A screenshot test, with shadows drawn as a phone or computer draws them:
/// tests otherwise draw them as solid blocks, which keeps checks steady, but
/// these are only for looking at.
void shots(
  String description,
  Future<void> Function(WidgetTester tester) body, {
  bool? skip,
}) => testWidgets(description, (tester) async {
  debugDisableShadows = false;
  try {
    await body(tester);
  } finally {
    debugDisableShadows = true;
  }
}, skip: skip);

void main() {
  setUpAll(loadFonts);

  for (final dark in [false, true]) {
    final b = dark ? 'dark' : 'light';
    final theme = dark ? ThemeMode.dark : ThemeMode.light;

    shots('sign in, $b', (tester) async {
      await size(tester, const Size(1280, 800));
      openEmpty();
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await shoot('sign-in-$b');
    });

    shots('unlock, vault, item, editor, desktop, $b', (tester) async {
      await size(tester, const Size(1280, 800));
      openShowcase();
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await shoot('unlock-$b');
      await unlock(tester);
      await tester.tap(find.text('Example Mail').first);
      await tester.pump();
      await tester.tap(find.byTooltip('Reveal').first);
      await tester.pump();
      await shoot('item-desktop-$b');
      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
      await tester.pumpAndSettle();
      await shoot('editor-desktop-$b');
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();

      if (Platform.environment['PB_SHOWCASE'] != null) {
        Future<void> open(String search, String name) async {
          await tester.enterText(find.byType(TextField).first, search);
          await tester.pump();
          await tester.tap(find.text(name).first);
          await tester.pump();
        }

        // An item made from a template, in an organization's shared folder.
        await open('office', 'Office server');
        await shoot('template-item-desktop-$b');
        // One in a shared folder that hides its passwords.
        await open('vpn', 'Fabrikam VPN');
        await shoot('hidden-passwords-desktop-$b');
        await tester.enterText(find.byType(TextField).first, '');
        await tester.pump();
      }

      // New item: the kinds, then More categories' templates.
      await tester.tap(find.text('New'));
      await tester.pumpAndSettle();
      await shoot('new-item-desktop-$b');
      await tester.tap(find.text('More categories'));
      await tester.pumpAndSettle();
      await shoot('new-item-more-desktop-$b');
      await tester.tap(find.text('Wi-Fi network'));
      await tester.pumpAndSettle();
      await shoot('editor-template-desktop-$b');
    });

    shots('unlocking with Windows Hello or Touch ID, $b', (tester) async {
      await size(tester, const Size(1280, 800));
      final program = Platform.environment['PB_TEST_AGENT']!;
      final home = Directory('/tmp').createTempSync('pb-ag-').path;
      final agent = (await tester.runAsync(
        () => Process.start(
          program,
          ['--serve'],
          environment: {
            'HOME': home,
            'PASSBUNDLE_AGENT_TEST_HOME': home,
            'PASSBUNDLE_AGENT_TEST_AUTHENTICATOR': home,
          },
        ),
      ))!;
      addTearDown(agent.kill);
      openSample();
      Core().useAgentHome(home, program);
      for (var i = 0; i < 100; i++) {
        if ((await tester.runAsync(Core.biometricStatus))!['status'] !=
            'noAgent') {
          break;
        }
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 100)),
        );
      }
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await unlock(tester);
      await tester.tap(find.byTooltip('Settings'));
      await tester.pump();
      await pumpUntil(tester, find.byKey(biometric));
      File('$home/answers').writeAsStringSync('yes');
      await tester.tap(find.byKey(biometric));
      await pumpUntil(
        tester,
        find.byWidgetPredicate(
          (w) => w is SwitchListTile && w.key == biometric && w.value,
        ),
      );
      // Past the dialog's fade-in, which the test's clock doesn't advance by itself.
      await tester.pump(const Duration(milliseconds: 400));
      await shoot('settings-$b');
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Lock'));
      await tester.pump();
      await pumpUntil(tester, find.textContaining('Unlock with '));
      await shoot('unlock-with-biometrics-$b');
    }, skip: Platform.environment['PB_TEST_AGENT'] == null);

    shots('vault, item, editor, and the offer, phone, $b', (tester) async {
      await size(tester, const Size(390, 844));
      PhoneStandIn(tester);
      openShowcase();
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await unlock(tester, phone: true);
      // Unlocked with the master password, on a phone with a fingerprint.
      await pumpUntil(tester, find.text('Unlock with fingerprint next time?'));
      await tester.pumpAndSettle();
      await shoot('unlock-offer-phone-$b');
      await tester.tap(find.text('Not now'));
      await tester.pumpAndSettle();
      await shoot('vault-phone-$b');
      // Its folders: Personal's, then each organization's shared folders.
      await tester.scrollUntilVisible(
        find.text('Folders'),
        200,
        scrollable: find.byType(Scrollable).last,
      );
      await tester.drag(find.byType(Scrollable).last, const Offset(0, -260));
      await tester.pumpAndSettle();
      await shoot('folders-phone-$b');
      await tester.drag(find.byType(Scrollable).last, const Offset(0, 4000));
      await tester.pumpAndSettle();

      await tester.enterText(find.byType(TextField), 'backup');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Backup service'));
      await tester.pumpAndSettle();
      await shoot('item-phone-$b');
      await tester.pageBack();
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('New item'));
      await tester.pumpAndSettle();
      await shoot('new-item-phone-$b');
      await tester.tap(find.text('Login'));
      await tester.pumpAndSettle();
      await shoot('editor-phone-$b');
    });

    for (final (width, text) in [(390.0, 1.0), (360.0, 1.3)]) {
      final name = text == 1
          ? 'settings-phone-$b'
          : 'settings-phone-${width.toInt()}-large-text-$b';
      shots(name, (tester) async {
        await size(tester, Size(width, 844), text: text);
        PhoneStandIn(tester, status: 'setUp');
        updates = Updates(_Updates());
        addTearDown(() => updates = null);
        openSample();
        await tester.pumpWidget(PassBundleApp(themeMode: theme));
        await unlock(tester, phone: true);
        await tester.tap(find.byTooltip('Settings'));
        await tester.pump();
        await pumpUntil(tester, find.byKey(biometric));
        await tester.pumpAndSettle();
        await shoot(name);
      });
    }
  }
}
