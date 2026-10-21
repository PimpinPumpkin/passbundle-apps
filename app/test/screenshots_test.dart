// Screenshots for people to look at, not checks: run with --update-goldens
// to (re)draw them in test/screenshots/. Desktop and phone width, light and dark.
@Tags(['screenshots'])
library;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

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
