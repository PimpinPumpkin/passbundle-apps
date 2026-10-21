// Screenshots for people to look at, not checks: run with --update-goldens
// to (re)draw them in test/screenshots/. Desktop and phone width, light and dark.
@Tags(['screenshots'])
library;

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';

import 'package:passbundle/main.dart';

import 'support.dart';

Future<void> size(WidgetTester tester, Size size) async {
  tester.view.physicalSize = size * 2;
  tester.view.devicePixelRatio = 2;
  addTearDown(tester.view.reset);
}

Future<void> shoot(String name) => expectLater(
  find.byType(MaterialApp),
  matchesGoldenFile('screenshots/$name.png'),
);

Future<void> unlock(WidgetTester tester) async {
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text('Backup service'));
  // The offline banner (no server behind the sample) settles first.
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
}

/// The switch for unlocking with Windows Hello or Touch ID, in Settings.
const biometric = ValueKey('biometric');

void main() {
  setUpAll(loadFonts);

  for (final dark in [false, true]) {
    final b = dark ? 'dark' : 'light';

    testWidgets('sign in, $b', (tester) async {
      await size(tester, const Size(1280, 800));
      openEmpty();
      await tester.pumpWidget(
        PassBundleApp(themeMode: dark ? ThemeMode.dark : ThemeMode.light),
      );
      await shoot('sign-in-$b');
    });

    testWidgets('unlock, vault, item, editor, desktop, $b', (tester) async {
      await size(tester, const Size(1280, 800));
      openSample();
      await tester.pumpWidget(
        PassBundleApp(themeMode: dark ? ThemeMode.dark : ThemeMode.light),
      );
      await shoot('unlock-$b');
      await unlock(tester);
      await tester.tap(find.text('Example Mail').first);
      await tester.pump();
      await tester.tap(find.byTooltip('Reveal').first);
      await tester.pump();
      await shoot('item-desktop-$b');
      await tester.tap(find.byTooltip('Edit'));
      await tester.pumpAndSettle();
      await shoot('editor-desktop-$b');
    });

    testWidgets('unlocking with Windows Hello or Touch ID, $b', (tester) async {
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
      await tester.pumpWidget(
        PassBundleApp(themeMode: dark ? ThemeMode.dark : ThemeMode.light),
      );
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

    testWidgets('vault, phone, $b', (tester) async {
      await size(tester, const Size(390, 844));
      openSample();
      await tester.pumpWidget(
        PassBundleApp(themeMode: dark ? ThemeMode.dark : ThemeMode.light),
      );
      await unlock(tester);
      await shoot('vault-phone-$b');
    });
  }
}
