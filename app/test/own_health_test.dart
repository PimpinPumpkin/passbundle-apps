// Members' own passwords in the apps (meta DESIGN.md, "Members' own
// passwords"): the core asks the server which organizations see how many of
// the person's own passwords are weak, reused, or exposed (`seen-by`) before
// it sends those numbers, and keeps the answer. The person is told who sees
// them, by name and in the web vault's words: on Health, on a computer and a
// phone, and in a phone's Settings; nothing is said while nobody does. The
// sample vault has no server behind it, so the core's test hook gives the
// names its last `seen-by` answer would have (core/tests/server.rs has the
// core asking a real server).
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/own_health.dart';
import 'package:passbundle/settings.dart';
import 'package:passbundle/vault.dart';

import 'launch_fixes_test.dart' show phone, setSize, unlock;
import 'support.dart';

const harbor =
    'Admins at Harbor Dental see how many of your own passwords are weak, '
    'reused, or exposed. Never which ones.';

/// The names the core says see the person's numbers, until the test ends.
void seenBy(List<String> names) {
  Core().testOwnHealthSeenBy(names);
  addTearDown(() => Core().testOwnHealthSeenBy(null));
}

Finder inMenu(Finder finder) =>
    find.descendant(of: find.byType(Sidebar), matching: finder);

/// A computer's window, tall enough for the whole menu.
const wide = Size(1280, 1100);

void main() {
  setUpAll(loadFonts);

  test('names the organizations in a sentence, as the web vault does', () {
    expect(seenByLine(const []), isNull);
    expect(seenByLine(const ['Harbor Dental']), harbor);
    expect(organizationNames(const ['Harbor Dental']), 'Harbor Dental');
    expect(
      organizationNames(const ['Harbor Dental', 'Northwind Clinic']),
      'Harbor Dental and Northwind Clinic',
    );
    expect(
      organizationNames(const ['Contoso', 'Fabrikam', 'Harbor Dental']),
      'Contoso, Fabrikam, and Harbor Dental',
    );
  });

  testWidgets(
    'a computer\'s Health says who sees how many of the person\'s own passwords are weak, even with nothing to fix',
    (tester) async {
      seenBy(const ['Harbor Dental']);
      await unlock(tester, wide);
      // Not on the other lists.
      expect(find.text(harbor), findsNothing);
      await tester.tap(inMenu(find.text('Health')));
      await tester.pumpAndSettle();
      // The sample's passwords are strong and used once: nothing to fix,
      // and the line all the same, over the list.
      expect(
        find.text('Nothing to fix: no weak, reused, or unsecured passwords.'),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('own-health-seen-by')), findsOneWidget);
      expect(find.text(harbor), findsOneWidget);
      await tester.tap(inMenu(find.text('All items')));
      await tester.pumpAndSettle();
      expect(find.text(harbor), findsNothing);
    },
  );

  testWidgets(
    'Health says nothing of it while no organization sees the numbers',
    (tester) async {
      await unlock(tester, wide);
      await tester.tap(inMenu(find.text('Health')));
      await tester.pumpAndSettle();
      expect(
        find.text('Nothing to fix: no weak, reused, or unsecured passwords.'),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('own-health-seen-by')), findsNothing);
      expect(find.textContaining('Admins at'), findsNothing);
    },
  );

  testWidgets(
    'on a phone, Health and Settings say who sees the numbers, by name',
    (tester) async {
      const both =
          'Admins at Harbor Dental and Northwind Clinic see how many of your '
          'own passwords are weak, reused, or exposed. Never which ones.';
      // Biometrics already on, so nothing is offered over the vault.
      PhoneStandIn(tester, status: 'setUp');
      seenBy(const ['Harbor Dental', 'Northwind Clinic']);
      setSize(tester, phone);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Logins'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      await tester.pumpAndSettle();
      expect(find.text(both), findsNothing);

      final card = find.byKey(const ValueKey('health'));
      await tester.ensureVisible(card);
      await tester.pumpAndSettle();
      await tester.tap(card);
      await tester.pumpAndSettle();
      expect(find.text(both), findsOneWidget);
      await tester.tap(find.byTooltip('Back to all items'));
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('Settings'));
      await tester.pump();
      await pumpUntil(tester, find.text('Lock after'));
      final row = find.byKey(const ValueKey('settings-own-health-seen-by'));
      await tester.scrollUntilVisible(
        row,
        200,
        scrollable: find
            .descendant(
              of: find.byType(SettingsPage),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      await tester.pumpAndSettle();
      expect(
        find.descendant(of: row, matching: find.text(both)),
        findsOneWidget,
      );
      expect(find.text('Your own passwords'), findsOneWidget);
    },
  );

  testWidgets(
    'a phone\'s Settings says nothing of it while no organization sees the numbers',
    (tester) async {
      PhoneStandIn(tester, status: 'setUp');
      setSize(tester, phone);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Logins'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Settings'));
      await tester.pump();
      await pumpUntil(tester, find.text('Lock after'));
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('settings-own-health-seen-by')),
        findsNothing,
      );
      expect(find.text('Your own passwords'), findsNothing);
    },
  );
}
