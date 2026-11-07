// The light or dark button beside Settings (widgets.dart, ThemeButton), as
// on the website: a moon while the app is light, a sun while it's dark, and
// a tap switches. Switching back to what the computer uses follows the
// computer again, so the button never pins a choice the person didn't need.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/widgets.dart';

void main() {
  test('a tap picks the other theme, or the computer\'s when that matches', () {
    expect(
      ThemeButton.next(dark: false, system: Brightness.light),
      ThemeMode.dark,
    );
    expect(
      ThemeButton.next(dark: true, system: Brightness.light),
      ThemeMode.system,
    );
    expect(
      ThemeButton.next(dark: false, system: Brightness.dark),
      ThemeMode.system,
    );
    expect(
      ThemeButton.next(dark: true, system: Brightness.dark),
      ThemeMode.light,
    );
  });

  Future<List<ThemeMode>> tapIn(WidgetTester tester, Brightness shown) async {
    final picked = <ThemeMode>[];
    await tester.pumpWidget(
      MaterialApp(
        theme: ThemeData(brightness: shown),
        home: Scaffold(
          body: Center(
            child: ThemeButton(onMode: picked.add),
          ),
        ),
      ),
    );
    await tester.tap(find.byType(ThemeButton));
    await tester.pump();
    return picked;
  }

  testWidgets('light: a moon that switches to dark', (tester) async {
    tester.platformDispatcher.platformBrightnessTestValue = Brightness.light;
    addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
    final picked = await tapIn(tester, Brightness.light);
    expect(find.byTooltip('Switch to dark mode'), findsOneWidget);
    expect(picked, [ThemeMode.dark]);
  });

  testWidgets('dark on a light computer: a sun back to following it', (
    tester,
  ) async {
    tester.platformDispatcher.platformBrightnessTestValue = Brightness.light;
    addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
    final picked = await tapIn(tester, Brightness.dark);
    expect(find.byTooltip('Switch to light mode'), findsOneWidget);
    expect(picked, [ThemeMode.system]);
  });

  testWidgets('dark on a dark computer: a sun that pins light', (tester) async {
    tester.platformDispatcher.platformBrightnessTestValue = Brightness.dark;
    addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
    final picked = await tapIn(tester, Brightness.dark);
    expect(picked, [ThemeMode.light]);
  });
}
