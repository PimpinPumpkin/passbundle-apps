// The prototype against the real core (PB_CORE_LIB): unlocking the sample
// vault, a wrong password, search, an item's view, revealing, and locking.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/theme.dart';

import 'support.dart';

void main() {
  setUpAll(loadFonts);
  tearDown(() => Core().lock());

  test('a wrong password is refused, in the core\'s words', () async {
    final answer = await Core.unlock(
      '{"profile":{"id":"x","email":"a@b.test","key":"2.AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}}',
      '{"kdf":0,"kdfIterations":600000}',
      'nope',
    );
    expect(answer['ok'], false);
    expect(answer['error'], "That master password isn't right.");
  });

  testWidgets(
    'the sample vault opens, searches, shows an item, reveals its password, and locks',
    (tester) async {
      tester.view.physicalSize = const Size(1280, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final answer = await unlockSample(tester);
      expect(answer['ok'], true);
      expect(answer['count'], 4);

      var locked = false;
      await tester.pumpWidget(
        MaterialApp(
          theme: passBundleTheme(Brightness.light),
          home: VaultScreen(
            core: Core(),
            mode: ThemeMode.light,
            onMode: (_) {},
            onLock: () => locked = true,
          ),
        ),
      );
      await tester.pump();
      for (final name in [
        'Backup service',
        'Example Mail',
        'Gate code',
        'Office Wi-Fi',
      ]) {
        expect(find.text(name), findsWidgets);
      }
      // Folders and organizations from the sync, with their real names.
      expect(find.text('Personal'), findsOneWidget);
      expect(find.text('Contoso'), findsOneWidget);

      await tester.enterText(
        find.widgetWithText(TextField, 'Search'),
        'backup',
      );
      await tester.pump();
      expect(find.text('Example Mail'), findsNothing);
      await tester.tap(find.text('Backup service'));
      await tester.pump();
      expect(find.text('backup@example.test'), findsWidgets);
      // The password stays in the core until revealed.
      expect(find.text('not-a-real-password-3'), findsNothing);
      await tester.tap(find.byTooltip('Reveal').first);
      await tester.pump();
      expect(find.text('not-a-real-password-3'), findsOneWidget);
      expect(find.textContaining(RegExp(r'^\d{3} \d{3}$')), findsOneWidget);

      await tester.tap(find.text('Lock'));
      expect(locked, true);
    },
  );

  testWidgets(
    'on a phone-width window, a header has the theme and Lock, and an item opens on its own page',
    (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await unlockSample(tester);
      var locked = false;
      await tester.pumpWidget(
        MaterialApp(
          theme: passBundleTheme(Brightness.light),
          home: VaultScreen(
            core: Core(),
            mode: ThemeMode.light,
            onMode: (_) {},
            onLock: () => locked = true,
          ),
        ),
      );
      await tester.pump();
      expect(find.text('Favorites'), findsNothing);
      await tester.tap(find.text('Office Wi-Fi'));
      await tester.pumpAndSettle();
      expect(find.text('Contoso'), findsOneWidget);
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Lock'));
      expect(locked, true);
    },
  );
}
