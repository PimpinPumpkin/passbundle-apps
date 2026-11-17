// A card or an address Android's autofill offered to save (docs/phones.md in
// the meta repo, "Autofill"; app/android's Saving.kt): the editor opens as a
// new card or identity, filled in with what the form held, as it does a new
// sign-in, and nothing is saved until Save. Which account it goes in is
// asked only among those where it may go (an organization's policy may
// remove the kind in some).
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/accounts.dart';
import 'package:passbundle/core.dart';
import 'package:passbundle/editor.dart';
import 'package:passbundle/places.dart';
import 'package:passbundle/theme.dart';

import 'launch_fixes_test.dart' show setSize;
import 'support.dart';

/// A button that runs [onPressed] with a context under the app's theme.
Future<void> pumpButton(
  WidgetTester tester,
  Future<void> Function(BuildContext context) onPressed,
) => tester.pumpWidget(
  MaterialApp(
    theme: passBundleTheme(Brightness.light),
    home: Builder(
      builder: (context) => Scaffold(
        body: TextButton(
          onPressed: () => onPressed(context),
          child: const Text('Open'),
        ),
      ),
    ),
  ),
);

/// The editor, opened as the vault opens it for what autofill offered to save.
Future<void> openFilledIn(
  WidgetTester tester,
  Map<String, dynamic> prefill,
) async {
  // Tall enough that every field is built.
  setSize(tester, const Size(390, 3000));
  await pumpButton(
    tester,
    (context) => openEditor(
      context,
      core: Core(),
      places: Places.empty,
      prefill: prefill,
      asDialog: false,
    ),
  );
  await tester.tap(find.text('Open'));
  await tester.pumpAndSettle();
}

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'a card autofill offered to save opens as a new card, filled in, and Cancel saves nothing',
    (tester) async {
      openEmpty();
      await openFilledIn(tester, {
        'kind': 'card',
        'name': 'Visa',
        'number': '4242424242424242',
        'brand': 'Visa',
        'expMonth': '7',
        'expYear': '2029',
        'cardholderName': 'Ana López',
        'code': '123',
        'mayAdd': ['a'],
      });
      expect(find.byType(ItemEditor), findsOneWidget);
      expect(find.text('New card'), findsOneWidget);
      // The name and the brand.
      expect(find.text('Visa'), findsNWidgets(2));
      for (final shown in ['Ana López', '4242424242424242', '07', '2029']) {
        expect(find.text(shown), findsOneWidget, reason: shown);
      }
      expect(find.text('123'), findsOneWidget);
      // A card's fields, not a login's.
      expect(find.text('Username'), findsNothing);
      expect(find.text('Cardholder name'), findsOneWidget);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
      expect(find.byType(ItemEditor), findsNothing);
    },
  );

  testWidgets(
    'an address autofill offered to save opens as a new identity, filled in',
    (tester) async {
      openEmpty();
      await openFilledIn(tester, {
        'kind': 'identity',
        'name': '1 Example Street',
        'firstName': 'Ana',
        'lastName': 'López',
        'address1': '1 Example Street',
        'address2': 'Apt 2',
        'city': 'Springfield',
        'state': 'IL',
        'postalCode': '62701',
        'email': 'ana@example.test',
      });
      expect(find.text('New identity'), findsOneWidget);
      // The name and the street.
      expect(find.text('1 Example Street'), findsNWidgets(2));
      for (final shown in [
        'Ana',
        'López',
        'Apt 2',
        'Springfield',
        'IL',
        '62701',
        'ana@example.test',
      ]) {
        expect(find.text(shown), findsOneWidget, reason: shown);
      }
      expect(find.text('Password'), findsNothing);
    },
  );

  testWidgets(
    'which account it goes in is asked only among those where it may go',
    (tester) async {
      final menu = AccountsMenu.from(
        {
          'accounts': [
            for (final (id, email) in [
              ('a', 'ana@work.example.test'),
              ('b', 'ana@home.example.test'),
              ('c', 'ana@club.example.test'),
            ])
              {
                'id': id,
                'email': email,
                'server': 'https://vault.example.test',
                'state': 'unlocked',
              },
          ],
          'active': 'a',
        },
        onSwitch: (_) {},
        onAdd: () {},
        onSignOut: () {},
      );
      String? chosen;
      List<String>? among;
      await pumpButton(
        tester,
        (context) async =>
            chosen = await chooseAccount(context, menu, among: among),
      );

      // Only one may take it: that one, without asking.
      among = ['b'];
      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();
      expect(find.text('Save it in which account?'), findsNothing);
      expect(chosen, 'b');

      // Two of three: only those two are offered.
      among = ['a', 'c'];
      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();
      expect(find.text('Save it in which account?'), findsOneWidget);
      expect(find.text('ana@work.example.test'), findsOneWidget);
      expect(find.text('ana@club.example.test'), findsOneWidget);
      expect(find.text('ana@home.example.test'), findsNothing);
      await tester.tap(find.text('ana@club.example.test'));
      await tester.pumpAndSettle();
      expect(chosen, 'c');

      // A login, as before: every unlocked account.
      among = null;
      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();
      for (final email in [
        'ana@work.example.test',
        'ana@home.example.test',
        'ana@club.example.test',
      ]) {
        expect(find.text(email), findsOneWidget);
      }
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(chosen, isNull);
    },
  );
}
