// The app against the real core (PB_CORE_LIB): signing in with nothing kept,
// unlocking the sample vault kept on this device, browsing, search, revealing,
// the editor, and locking.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/main.dart';

import 'support.dart';

void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'with nothing kept, it asks to sign in, and says when the server can\'t be reached',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      openEmpty();
      await tester.pumpWidget(const PassBundleApp());
      expect(find.text('Sign in to PassBundle'), findsOneWidget);
      // PassBundle's own server unless someone hosts their own.
      expect(find.widgetWithText(TextField, 'Server'), findsNothing);
      await tester.tap(find.text('Self-hosted?'));
      await tester.pump();
      await tester.enterText(
        find.widgetWithText(TextField, 'Server'),
        'http://127.0.0.1:9',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Email'),
        'ana@example.test',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        'anything',
      );
      await tester.tap(find.text('Sign in'));
      await pumpUntil(
        tester,
        find.textContaining('Couldn\'t reach the server'),
      );
      // The field doesn't keep the master password.
      expect(find.text('anything'), findsNothing);

      await tester.enterText(
        find.widgetWithText(TextField, 'Server'),
        'http://vault.example.test',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        'anything',
      );
      await tester.tap(find.text('Sign in'));
      await pumpUntil(
        tester,
        find.textContaining('only talks to servers over HTTPS'),
      );
    },
  );

  testWidgets(
    'signing in asks for a server only for someone hosting their own',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      openEmpty();
      await tester.pumpWidget(const PassBundleApp());
      expect(find.text('Sign in to PassBundle'), findsOneWidget);
      expect(find.widgetWithText(TextField, 'Server'), findsNothing);
      expect(find.widgetWithText(TextField, 'Email'), findsOneWidget);
      await tester.tap(find.text('Self-hosted?'));
      await tester.pump();
      expect(find.widgetWithText(TextField, 'Server'), findsOneWidget);
      await tester.tap(find.text('Use PassBundle\'s server'));
      await tester.pump();
      expect(find.widgetWithText(TextField, 'Server'), findsNothing);
      // No organization set one by policy on this machine.
      expect(find.textContaining('Your organization\'s server'), findsNothing);
    },
  );

  testWidgets(
    'the kept vault unlocks, browses, searches, reveals, edits, and locks',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      expect(find.text('Unlock PassBundle'), findsOneWidget);
      expect(find.text('ana.lopez@example.test'), findsOneWidget);

      await tester.enterText(find.byType(TextField), 'not the password');
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('That master password isn\'t right.'));

      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Backup service'));
      // No refresh token kept here, so syncing asks to sign in again; the vault stays open.
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      for (final name in [
        'Example Mail',
        'Gate code',
        'Office Wi-Fi',
        'Personal',
        'Contoso',
      ]) {
        expect(find.text(name), findsWidgets);
      }

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

      // The editor opens with the item's values; saving without a server says so.
      await tester.tap(find.byTooltip('Edit'));
      await tester.pumpAndSettle();
      expect(find.text('Edit item'), findsOneWidget);
      expect(find.widgetWithText(TextField, 'Backup service'), findsOneWidget);
      // The generator: a password, then a passphrase, which it remembers.
      await tester.tap(find.byTooltip('Generate a password'));
      await tester.pumpAndSettle();
      String generated() => tester
          .widget<SelectableText>(find.byKey(const Key('generated')))
          .data!;
      expect(generated(), hasLength(20));
      await tester.tap(find.text('0-9'));
      await tester.pumpAndSettle();
      expect(generated(), hasLength(20));
      expect(generated(), isNot(matches(RegExp(r'\d'))));
      await tester.tap(find.text('Passphrase'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.widgetWithText(TextField, 'Between words'),
        ' ',
      );
      await tester.tap(find.text('Capitalize'));
      await tester.pumpAndSettle();
      final passphrase = generated();
      expect(passphrase.split(' '), hasLength(6));
      expect(passphrase, matches(RegExp(r'^([A-Z][a-z-]+ ){5}[A-Z][a-z-]+$')));
      await tester.tap(find.text('Use this passphrase'));
      await tester.pumpAndSettle();
      expect(find.text('Generator'), findsNothing);
      expect(find.widgetWithText(TextField, passphrase), findsOneWidget);
      await tester.tap(find.byTooltip('Generate a password'));
      await tester.pumpAndSettle();
      expect(find.text('Use this passphrase'), findsOneWidget);
      expect(generated().split(' '), hasLength(6));
      await tester.tap(find.text('Cancel').last);
      await tester.pumpAndSettle();
      expect(find.widgetWithText(TextField, passphrase), findsOneWidget);

      await tester.tap(find.text('Save'));
      await pumpUntil(
        tester,
        find.text('You\'ve been signed out. Sign in again.'),
      );
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();

      await tester.tap(find.text('Lock'));
      await tester.pump();
      expect(find.text('Unlock PassBundle'), findsOneWidget);
      expect(find.text('Backup service'), findsNothing);
    },
  );

  testWidgets(
    'the generator holds to an organization\'s Password generator policy',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      openSample(
        policies: [
          {
            'id': 'p-generator',
            'organizationId': 'o',
            'type': 2,
            'enabled': true,
            'data': {
              'overridePasswordType': 'passphrase',
              'minNumberWords': 8,
              'capitalize': true,
            },
            'object': 'policy',
          },
        ],
      );
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Backup service'));
      // Syncing finishes (it asks to sign in again) before anything settles.
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      await tester.tap(find.text('Backup service'));
      await tester.pump();
      await tester.tap(find.byTooltip('Edit'));
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('Generate a password'));
      await tester.pumpAndSettle();
      expect(
        find.text('Your organization sets some of these.'),
        findsOneWidget,
      );
      // A passphrase only, of at least 8 capitalized words.
      expect(find.text('Use this passphrase'), findsOneWidget);
      await tester.tap(
        find.descendant(
          of: find.byType(SegmentedButton<String>),
          matching: find.text('Password'),
        ),
        warnIfMissed: false,
      );
      await tester.pumpAndSettle();
      expect(find.text('Use this passphrase'), findsOneWidget);
      await tester.enterText(
        find.widgetWithText(TextField, 'Between words'),
        ' ',
      );
      await tester.pumpAndSettle();
      final made = tester
          .widget<SelectableText>(find.byKey(const Key('generated')))
          .data!;
      expect(made.split(' '), hasLength(8));
      expect(made, matches(RegExp(r'^([A-Z][a-z-]+ ){7}[A-Z][a-z-]+$')));
      expect(find.text('8'), findsOneWidget);
      final capitalize = tester.widget<SwitchListTile>(
        find.widgetWithText(SwitchListTile, 'Capitalize'),
      );
      expect(capitalize.value, isTrue);
      expect(capitalize.onChanged, isNull);
      await tester.tap(find.text('Cancel').last);
      await tester.pumpAndSettle();
    },
  );

  testWidgets(
    'a card and an identity show their details, with their secrets hidden until revealed, and the editor has them',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Everyday card'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      // Lists show a card by its brand and last 4 digits.
      expect(find.text('Visa, *1111'), findsOneWidget);

      await tester.tap(find.text('Everyday card'));
      await tester.pump();
      expect(find.text('Ana Lopez'), findsOneWidget);
      expect(find.text('Visa'), findsOneWidget);
      expect(find.text('07/2029'), findsOneWidget);
      expect(find.text('4111111111111111'), findsNothing);
      await tester.tap(find.byTooltip('Reveal').first);
      await tester.pump();
      expect(find.text('4111111111111111'), findsOneWidget);
      expect(find.text('123'), findsNothing);
      // The editor has its details, the secrets hidden until shown.
      await tester.tap(find.byTooltip('Edit'));
      await tester.pumpAndSettle();
      expect(find.widgetWithText(TextField, 'Ana Lopez'), findsOneWidget);
      expect(find.widgetWithText(TextField, '2029'), findsOneWidget);
      expect(
        find.widgetWithText(TextField, '4111111111111111'),
        findsOneWidget,
      );
      expect(find.text('Visa'), findsWidgets);
      expect(find.text('07'), findsOneWidget);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();

      await tester.tap(find.text('Ana at home'));
      await tester.pump();
      expect(find.text('Ms Ana María López'), findsWidgets);
      expect(find.text('ana.lopez@example.test'), findsWidgets);
      expect(
        find.text('1 Example Street\nApartment 2\nSpringfield, IL 62701\nUS'),
        findsOneWidget,
      );
      expect(find.text('123-45-6789'), findsNothing);
      await tester.tap(find.byTooltip('Reveal').first);
      await tester.pump();
      expect(find.text('123-45-6789'), findsOneWidget);
      await tester.tap(find.byTooltip('Edit'));
      await tester.pumpAndSettle();
      expect(find.widgetWithText(TextField, 'María'), findsOneWidget);
      for (final value in ['Springfield', 'X0000000']) {
        await tester.dragUntilVisible(
          find.widgetWithText(TextField, value),
          find.byType(ListView).last,
          const Offset(0, -200),
        );
        expect(find.widgetWithText(TextField, value), findsOneWidget);
      }
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
    },
  );

  testWidgets(
    'an item that asks for the master password again shows nothing secret until it\'s given',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      // "Backup service", as the web vault sets it.
      openSample(asking: {'5f607182-93a4-4b52-cde3-f40516273849'});
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Backup service'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      await tester.tap(find.text('Backup service'));
      await tester.pump();
      expect(
        find.text('This item asks for your master password.'),
        findsOneWidget,
      );
      expect(find.byTooltip('Reveal'), findsNothing);
      expect(find.byTooltip('Edit'), findsNothing);

      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        'not the password',
      );
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.text('That master password isn\'t right.'));
      expect(find.byTooltip('Reveal'), findsNothing);

      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        samplePassword,
      );
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.byTooltip('Edit'));
      await tester.tap(find.byTooltip('Reveal').first);
      await tester.pump();
      expect(find.text('not-a-real-password-3'), findsOneWidget);

      // Other items don't ask.
      await tester.tap(find.text('Example Mail'));
      await tester.pump();
      expect(find.byTooltip('Reveal'), findsWidgets);
    },
  );

  testWidgets(
    'an SSH key shows its public key and fingerprint, and its private key once revealed',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Build server'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      await tester.tap(find.text('Build server'));
      await tester.pump();
      expect(find.textContaining('ssh-ed25519 '), findsOneWidget);
      expect(find.textContaining('SHA256:'), findsWidgets);
      expect(find.textContaining('PRIVATE KEY'), findsNothing);
      await tester.tap(find.byTooltip('Reveal').first);
      await tester.pump();
      expect(find.textContaining('BEGIN OPENSSH PRIVATE KEY'), findsOneWidget);
      // The web vault edits SSH keys.
      expect(find.byTooltip('Edit'), findsNothing);
    },
  );

  testWidgets(
    'on a phone-width window, a header has the theme and Lock, and an item opens on its own page',
    (tester) async {
      setSize(tester, const Size(390, 844));
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Office Wi-Fi'));
      // The background sync finishes (offline: no server behind the sample).
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      expect(find.text('Favorites'), findsNothing);
      await tester.tap(find.text('Office Wi-Fi'));
      await tester.pumpAndSettle();
      expect(find.text('Contoso'), findsOneWidget);
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Lock'));
      await tester.pump();
      expect(find.text('Unlock PassBundle'), findsOneWidget);
    },
  );
}
