// Expiration dates in the app, against the real core (core/src/expiration.rs):
// a badge on what's expired or expires within 30 days, Expiring soon in the
// menu, Health counting them, the reminder at the top of the vault (how
// many, never which), the item's date and reminder days, and the editor's
// Expires and Remind me. The sample vault is encrypted, so its items are
// given dates by the core's test hook, as if the web vault had set them.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/vault.dart';

import 'support.dart';

/// The sample's own login (the core's vectors).
const exampleMail = '3d4e5f60-7182-4930-abc1-d2e3f4051627';

void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// The sample vault, unlocked, with these items given dates (by id:
/// `expires`, and `remind` if any), once its offline sync has given up.
Future<void> unlock(
  WidgetTester tester,
  Size size,
  Map<String, Map<String, String>> expiring, {
  required String ready,
}) async {
  setSize(tester, size);
  openSample();
  Core().testExpiration(expiring);
  addTearDown(() => Core().testExpiration(const {}));
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text(ready));
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
}

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'what expires soon has a badge, a place in the menu, and a count in Health, and a reminder says how many',
    (tester) async {
      await unlock(tester, const Size(1280, 1100), {
        exampleMail: {'expires': isoInDays(5)},
        gateCode: {'expires': isoInDays(-5), 'remind': '7,1'},
      }, ready: 'Example Mail');
      // Said once, at the top: how many, never which.
      const said = '2 items expire soon or have expired';
      await pumpUntil(tester, find.text(said));
      expect(find.byKey(const ValueKey('expiration-reminder')), findsOneWidget);

      // Each list row's badge: amber soon, the error color once expired.
      expect(find.text('Expires in 5 days'), findsOneWidget);
      expect(find.text('Expired 5 days ago'), findsOneWidget);
      expect(find.byKey(const ValueKey('expiry-badge')), findsNWidgets(2));

      final menu = find.byType(Sidebar);
      Finder entry(String label) =>
          find.descendant(of: menu, matching: find.text(label));
      expect(entry('Expiring soon'), findsOneWidget);
      // The sample's passwords are strong and used once: Health is these two.
      await pumpUntil(tester, entry('2 to fix'));

      // Show opens Expiring soon, and the reminder goes.
      await tester.tap(find.text('Show'));
      await tester.pumpAndSettle();
      expect(find.text(said), findsNothing);
      expect(find.text('Example Mail'), findsOneWidget);
      expect(find.text('Gate code'), findsOneWidget);
      expect(find.text('Backup service'), findsNothing);
      expect(find.text('Everyday card'), findsNothing);

      // Health says what each has to fix.
      await tester.tap(entry('Health'));
      await tester.pumpAndSettle();
      expect(find.text('Expires in 5 days'), findsNWidgets(2));
      expect(find.text('Backup service'), findsNothing);

      // The item: its date, how far off, and its own reminder days.
      await tester.tap(find.text('Gate code'));
      await tester.pumpAndSettle();
      expect(
        find.text('${isoInDays(-5)} · Expired 5 days ago'),
        findsOneWidget,
      );
      expect(find.text('7 and 1 day before'), findsOneWidget);
      await tester.tap(find.text('Example Mail').first);
      await tester.pumpAndSettle();
      expect(find.text('${isoInDays(5)} · Expires in 5 days'), findsOneWidget);
      // The default days aren't said.
      expect(find.text('Remind me'), findsNothing);
    },
  );

  testWidgets(
    'the editor shows the date and reminder days, offers its choices, and refuses a date that isn\'t one',
    (tester) async {
      await unlock(tester, const Size(1280, 1100), {
        gateCode: {'expires': isoInDays(20), 'remind': '7,1'},
      }, ready: 'Gate code');
      await tester.tap(find.text('Gate code'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
      await tester.pumpAndSettle();

      final expires = find.widgetWithText(TextField, 'Expires (optional)');
      await tester.ensureVisible(expires);
      await tester.pumpAndSettle();
      expect(find.widgetWithText(TextField, isoInDays(20)), findsOneWidget);
      final remind = find.byKey(const ValueKey('remind-me'));
      expect(
        find.descendant(of: remind, matching: find.text('7 and 1 day before')),
        findsOneWidget,
      );
      await tester.tap(remind);
      await tester.pumpAndSettle();
      for (final choice in [
        '30, 7, and 1 day before',
        '90, 30, 7, and 1 day before',
        "Don't remind me",
      ]) {
        expect(find.text(choice), findsWidgets);
      }
      await tester.tap(find.text("Don't remind me").last);
      await tester.pumpAndSettle();

      // No date, no reminder days.
      await tester.tap(find.byTooltip('No expiration date'));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('remind-me')), findsNothing);

      // A date that isn't one is refused before anything is sent.
      await tester.enterText(expires, '2027-02-30');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Save'));
      await pumpUntil(
        tester,
        find.textContaining('write the expiration date as year, month, and day'),
      );
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
    },
  );

  testWidgets(
    'on a phone, Expiring soon is a card above the favorites, and the reminder can be dismissed',
    (tester) async {
      await unlock(tester, const Size(390, 844), {
        exampleMail: {'expires': isoInDays(1)},
      }, ready: 'Logins');
      await pumpUntil(tester, find.text('1 item expires soon'));
      await tester.tap(find.text('Dismiss'));
      await tester.pumpAndSettle();
      expect(find.text('1 item expires soon'), findsNothing);

      final card = find.byKey(const ValueKey('expiring-soon'));
      expect(card, findsOneWidget);
      expect(
        find.descendant(of: card, matching: find.text('1')),
        findsOneWidget,
      );
      await tester.tap(card);
      await tester.pumpAndSettle();
      expect(find.text('Example Mail'), findsOneWidget);
      expect(find.text('Expires tomorrow'), findsOneWidget);
      expect(find.text('Backup service'), findsNothing);
    },
  );

  testWidgets('nothing expiring: no reminder, no badge, no Expiring soon', (
    tester,
  ) async {
    await unlock(tester, const Size(1280, 1100), {
      // Further off than any reminder day.
      exampleMail: {'expires': isoInDays(45)},
    }, ready: 'Example Mail');
    await tester.pump(const Duration(seconds: 2));
    expect(find.byKey(const ValueKey('expiration-reminder')), findsNothing);
    expect(find.byKey(const ValueKey('expiry-badge')), findsNothing);
    expect(
      find.descendant(
        of: find.byType(Sidebar),
        matching: find.text('Expiring soon'),
      ),
      findsNothing,
    );
  });
}
