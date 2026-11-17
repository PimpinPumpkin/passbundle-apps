// An organization's "Remove card item type" policy (core/src/restricted_types.rs):
// New doesn't offer a kind it removes, on a computer and a phone, and the
// person's own card isn't listed. Against the real core, with the sample
// vault's kept sync carrying the policy, as the server sends it.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/editor.dart' show ItemEditor;
import 'package:passbundle/main.dart';
import 'package:passbundle/new_item.dart';
import 'package:passbundle/theme.dart';

import 'launch_fixes_test.dart' show computer, phone, setSize;
import 'support.dart';

/// Contoso's policy, on, with no list: cards. The person owns Contoso, the
/// sample's organization, and the policy binds owners too.
const removesCards = <String, dynamic>{
  'id': 'p-cards',
  'organizationId': '8f9a7c1e-3b2d-4e5f-9a8b-7c6d5e4f3a2b',
  'type': 15,
  'enabled': true,
  'data': null,
  'object': 'policy',
};

void main() {
  setUpAll(loadFonts);

  for (final (on, size) in [('a computer', computer), ('a phone', phone)]) {
    testWidgets(
      'on $on, New doesn\'t offer a card where an organization removes cards, and the person\'s own card isn\'t listed',
      (tester) async {
        setSize(tester, size);
        openSample(policies: [removesCards]);
        await tester.pumpWidget(const PassBundleApp());
        await tester.enterText(find.byType(TextField), samplePassword);
        await tester.tap(find.text('Unlock'));
        await pumpUntil(tester, find.text('Example Mail'));
        await pumpUntil(
          tester,
          find.textContaining('Sign in again to get changes'),
        );
        expect(find.text('Everyday card'), findsNothing);

        await tester.tap(find.byTooltip('New item'));
        await tester.pumpAndSettle();
        expect(find.text('What are you saving?'), findsOneWidget);
        expect(find.text('Credit or debit card'), findsNothing);
        for (final offered in [
          'Website or app',
          'Important text',
          'Personal info',
        ]) {
          expect(find.text(offered), findsOneWidget);
        }
        // The others open their editor as before.
        await tester.tap(find.text('Personal info'));
        await tester.pumpAndSettle();
        expect(find.byType(ItemEditor), findsOneWidget);
        await tester.tap(find.byTooltip('Cancel'));
        await tester.pumpAndSettle();
        expect(find.byType(ItemEditor), findsNothing);
      },
    );
  }

  testWidgets(
    'the picker leaves out every kind it\'s told to, the web vault\'s too, and the rest still fill their rows',
    (tester) async {
      setSize(tester, phone);
      NewItemChoice? chosen;
      await tester.pumpWidget(
        MaterialApp(
          theme: passBundleTheme(Brightness.light),
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () async => chosen = await chooseNewItem(
                  context,
                  templates: const [],
                  asDialog: false,
                  restricted: const {'card', 'sshKey'},
                ),
                child: const Text('Open'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();
      expect(find.text('What are you saving?'), findsOneWidget);
      expect(find.text('Credit or debit card'), findsNothing);
      // Two to a row on a phone: Login and Note, then Identity alone.
      final login = tester.getRect(find.text('Website or app'));
      final note = tester.getRect(find.text('Important text'));
      final identity = tester.getRect(find.text('Personal info'));
      expect(note.top, login.top);
      expect(identity.top, greaterThan(login.bottom));
      expect(identity.left, login.left);

      await tester.tap(find.text('More categories'));
      await tester.pumpAndSettle();
      expect(find.text('SSH key'), findsNothing);
      for (final kind in ['Bank account', 'Driver\'s license', 'Passport']) {
        expect(find.text(kind), findsOneWidget);
      }

      await tester.tap(find.text('Personal info'));
      await tester.pumpAndSettle();
      expect(chosen?.kind, 'identity');
    },
  );
}
