// Screenshots for people to look at, not checks: run with --update-goldens
// to (re)draw them in test/screenshots/. Desktop and phone width, light and dark.
@Tags(['screenshots'])
library;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/theme.dart';

import 'support.dart';

Future<void> shoot(
  WidgetTester tester,
  String name,
  Size size,
  Brightness brightness,
  Widget home,
) async {
  tester.view.physicalSize = size * 2;
  tester.view.devicePixelRatio = 2;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: passBundleTheme(brightness),
      home: home,
    ),
  );
  await tester.pump();
  await expectLater(
    find.byType(MaterialApp),
    matchesGoldenFile('screenshots/$name.png'),
  );
}

void main() {
  setUpAll(loadFonts);
  tearDown(() => Core().lock());

  for (final brightness in Brightness.values) {
    final b = brightness.name;
    testWidgets('unlock, $b', (tester) async {
      await shoot(
        tester,
        'unlock-$b',
        const Size(1280, 800),
        brightness,
        UnlockScreen(onUnlocked: () {}),
      );
    });

    testWidgets('vault, desktop, $b', (tester) async {
      await unlockSample(tester);
      final core = Core();
      await shoot(
        tester,
        'vault-desktop-$b',
        const Size(1280, 800),
        brightness,
        VaultScreen(
          core: core,
          mode: ThemeMode.light,
          onMode: (_) {},
          onLock: () {},
        ),
      );
      await tester.tap(find.text('Example Mail').first);
      await tester.pump();
      await tester.tap(find.byTooltip('Reveal').first);
      await tester.pump();
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile('screenshots/item-desktop-$b.png'),
      );
    });

    testWidgets('vault, phone, $b', (tester) async {
      await unlockSample(tester);
      await shoot(
        tester,
        'vault-phone-$b',
        const Size(390, 844),
        brightness,
        VaultScreen(
          core: Core(),
          mode: ThemeMode.light,
          onMode: (_) {},
          onLock: () {},
        ),
      );
    });
  }
}
