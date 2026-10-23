// The app against the real core (PB_CORE_LIB): signing in with nothing kept,
// unlocking the sample vault kept on this device, browsing, search, revealing,
// the editor, and locking.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/editor.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/new_item.dart';
import 'package:passbundle/vault.dart';

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
    'making an account checks the master password here, then goes to the server it would sign in to',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      openEmpty();
      await tester.pumpWidget(const PassBundleApp());
      await tester.tap(find.text('Create account'));
      await tester.pump();
      expect(find.text('Create a PassBundle account'), findsOneWidget);
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
      Future<void> passwords(String one, String two, [String hint = '']) async {
        await tester.enterText(
          find.widgetWithText(TextField, 'Master password'),
          one,
        );
        await tester.enterText(
          find.widgetWithText(TextField, 'Master password again'),
          two,
        );
        await tester.enterText(
          find.widgetWithText(TextField, 'Hint (optional)'),
          hint,
        );
        await tester.tap(find.text('Create account'));
        await tester.pump();
      }

      await passwords('too short', 'too short');
      expect(find.text('Use at least 12 characters.'), findsOneWidget);
      await passwords('correct horse battery', 'correct horse battery!');
      expect(
        find.text('The two master passwords don\'t match.'),
        findsOneWidget,
      );
      await passwords(
        'correct horse battery',
        'correct horse battery',
        'it\'s Correct Horse Battery',
      );
      expect(
        find.text('The hint can\'t give away your master password.'),
        findsOneWidget,
      );
      // Right: the core makes the account's keys, then can't reach the server.
      await passwords('correct horse battery', 'correct horse battery');
      await pumpUntil(
        tester,
        find.textContaining('Couldn\'t reach the server'),
      );
      // The fields don't keep the master password.
      expect(find.text('correct horse battery'), findsNothing);

      await tester.tap(find.text('I have an account: sign in'));
      await tester.pump();
      expect(find.text('Sign in to PassBundle'), findsOneWidget);
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
        find.widgetWithText(TextField, 'Search items'),
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
      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
      await tester.pumpAndSettle();
      expect(find.text('Edit login'), findsOneWidget);
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
      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
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
      // The generator's count, not the one-time code's seconds behind it.
      expect(
        tester.widget<Text>(find.byKey(const Key('Words count'))).data,
        '8',
      );
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
      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
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
      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
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
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsNothing);

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
      await pumpUntil(tester, find.widgetWithText(OutlinedButton, 'Edit'));
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
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsNothing);
    },
  );

  testWidgets('a login shows its passkey: who it\'s for, never its key', (
    tester,
  ) async {
    setSize(tester, const Size(1280, 800));
    openSample();
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
    expect(find.text('Passkey'), findsOneWidget);
    // Made at 18:00 UTC: the 20th, or the 21st east of UTC+6.
    expect(
      find.textContaining(
        RegExp(r'^For Backup Service · created October 2[01], 2026$'),
      ),
      findsOneWidget,
    );
    // A login without one shows none.
    await tester.tap(find.text('Example Mail'));
    await tester.pump();
    expect(find.text('Passkey'), findsNothing);
  });

  testWidgets(
    'an item lists its attachments, and saving one needs the server',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Example Mail'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      await tester.tap(find.text('Example Mail'));
      await tester.pump();
      expect(find.text('recovery-codes.txt · 36 bytes'), findsOneWidget);
      // The sample has no server: downloading it says so.
      await tester.tap(find.byTooltip('Save recovery-codes.txt'));
      await pumpUntil(
        tester,
        find.text('You\'ve been signed out. Sign in again.'),
      );
    },
  );

  testWidgets(
    'folders are made, renamed, and deleted from the sidebar, which needs the server',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Example Mail'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );

      // A new folder needs a name.
      await tester.tap(find.byTooltip('New folder'));
      await tester.pumpAndSettle();
      final save = find.widgetWithText(FilledButton, 'Save');
      expect(tester.widget<FilledButton>(save).onPressed, isNull);
      await tester.enterText(find.widgetWithText(TextField, 'Name'), '  ');
      await tester.pump();
      expect(tester.widget<FilledButton>(save).onPressed, isNull);
      await tester.enterText(find.widgetWithText(TextField, 'Name'), 'Bills');
      await tester.pump();
      // The sample has no server: making it says so.
      await tester.tap(save);
      await pumpUntil(
        tester,
        find.text('You\'ve been signed out. Sign in again.'),
      );
      // The dialog closes; no folder was made.
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text('Bills'), findsNothing);

      // Renaming starts from its name.
      await tester.tap(find.byTooltip('More for Personal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Rename'));
      await tester.pumpAndSettle();
      expect(find.text('Rename folder'), findsOneWidget);
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Name'))
            .controller!
            .text,
        'Personal',
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      // Deleting asks first, and says its items stay.
      await tester.tap(find.byTooltip('More for Personal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();
      expect(find.text('Delete the folder Personal?'), findsOneWidget);
      expect(
        find.text('Its items stay in your vault, in no folder.'),
        findsOneWidget,
      );
      await tester.tap(find.text('Delete folder'));
      await pumpUntil(
        tester,
        find.text('You\'ve been signed out. Sign in again.'),
      );
      expect(find.byTooltip('More for Personal'), findsOneWidget);
    },
  );

  Future<void> unlockOnComputer(
    WidgetTester tester, {
    Map<String, Map<String, dynamic>> changed = const {},
    Size size = const Size(1280, 800),
  }) async {
    setSize(tester, size);
    openSample(changed: changed);
    await tester.pumpWidget(const PassBundleApp());
    await tester.enterText(find.byType(TextField), samplePassword);
    await tester.tap(find.text('Unlock'));
    await pumpUntil(tester, find.text('Example Mail'));
    await pumpUntil(
      tester,
      find.textContaining('Sign in again to get changes'),
    );
  }

  /// The sample's organization item and its shared folder (the core's vectors).
  const officeWifi = '4e5f6071-8293-4a41-bcd2-e3f405162738';
  const it = '2c3d4e5f-6071-4829-9ab0-c1d2e3f40516';

  testWidgets(
    'the menu\'s folders are Personal\'s, then each organization\'s shared folders, each with its count, and each shows its items',
    (tester) async {
      // Tall enough for the whole menu.
      await unlockOnComputer(tester, size: const Size(1280, 1100));
      final menu = find.byType(Sidebar);
      Finder entry(String label) =>
          find.descendant(of: menu, matching: find.text(label));
      // In order: Personal, its folder (called Personal too), No folder,
      // then Contoso and its IT.
      double top(Finder f) => tester.getTopLeft(f).dy;
      final personal = entry('Personal');
      expect(personal, findsNWidgets(2));
      expect(top(personal.first), lessThan(top(personal.last)));
      expect(top(personal.last), lessThan(top(entry('No folder'))));
      expect(top(entry('No folder')), lessThan(top(entry('Contoso'))));
      expect(top(entry('Contoso')), lessThan(top(entry('IT'))));
      // A folder sits a step in from its owner.
      expect(
        tester.getTopLeft(personal.last).dx,
        greaterThan(tester.getTopLeft(personal.first).dx),
      );

      Future<void> pick(Finder f) async {
        await tester.ensureVisible(f);
        await tester.pumpAndSettle();
        await tester.tap(f);
        await tester.pump();
      }

      await pick(entry('IT'));
      expect(find.text('Office Wi-Fi'), findsOneWidget);
      expect(find.text('Example Mail'), findsNothing);
      await pick(entry('No folder'));
      expect(find.text('Gate code'), findsOneWidget);
      expect(find.text('Example Mail'), findsNothing);
      expect(find.text('Office Wi-Fi'), findsNothing);
      await pick(personal.first);
      expect(find.text('Example Mail'), findsOneWidget);
      expect(find.text('Office Wi-Fi'), findsNothing);
      // Where each item is, beside it in the list.
      await pick(entry('All items'));
      expect(find.widgetWithText(WhereBadge, 'IT'), findsOneWidget);
    },
  );

  testWidgets(
    'an item whose shared folder hides passwords shows none of its secrets, and offers no edit or trash',
    (tester) async {
      await unlockOnComputer(
        tester,
        changed: {
          officeWifi: {
            'edit': false,
            'viewPassword': false,
            'permissions': {'delete': false, 'restore': false},
          },
          it: {'readOnly': true, 'hidePasswords': true},
        },
      );
      await tester.tap(find.text('Office Wi-Fi'));
      await tester.pump();
      expect(
        find.text('Your organization hides this item\'s passwords from you.'),
        findsOneWidget,
      );
      expect(find.byTooltip('Reveal'), findsNothing);
      expect(find.byTooltip('Copy password'), findsNothing);
      expect(find.byTooltip('Copy username'), findsOneWidget);
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsNothing);
      expect(find.byTooltip('Move to trash'), findsNothing);
      // The person's own items are as ever.
      await tester.tap(find.text('Example Mail'));
      await tester.pump();
      expect(find.byTooltip('Reveal'), findsWidgets);
      expect(find.widgetWithText(OutlinedButton, 'Edit'), findsOneWidget);
      expect(find.byTooltip('Move to trash'), findsOneWidget);
    },
  );

  testWidgets(
    'New item offers the main kinds, then templates, and a template\'s editor starts with its fields',
    (tester) async {
      await unlockOnComputer(tester);
      await tester.tap(find.text('New'));
      await tester.pumpAndSettle();
      expect(find.text('What are you saving?'), findsOneWidget);
      for (final kind in ['Login', 'Secure note', 'Card', 'Identity']) {
        expect(find.text(kind), findsOneWidget);
      }
      expect(find.text('Wi-Fi network'), findsNothing);
      await tester.tap(find.text('More categories'));
      await tester.pumpAndSettle();
      for (final t in ['API key', 'Server', 'Domain', 'Software license']) {
        expect(find.text(t), findsOneWidget);
      }
      await tester.enterText(
        find.widgetWithText(TextField, 'Find a category'),
        'wi-fi',
      );
      await tester.pumpAndSettle();
      expect(find.text('Server'), findsNothing);
      await tester.tap(find.text('Wi-Fi network'));
      await tester.pumpAndSettle();
      expect(find.text('New Wi-Fi network'), findsOneWidget);
      for (final field in ['Network name', 'Password', 'Security type']) {
        expect(find.widgetWithText(TextField, field), findsOneWidget);
      }
      // A note: no username or websites.
      expect(find.widgetWithText(TextField, 'Username'), findsNothing);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();

      // A domain's yes-or-no field is a switch.
      await tester.tap(find.text('New'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('More categories'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Domain'));
      await tester.pumpAndSettle();
      expect(find.text('New domain'), findsOneWidget);
      expect(find.widgetWithText(SwitchListTile, 'Auto-renew'), findsOneWidget);
      expect(find.widgetWithText(TextField, 'Username'), findsOneWidget);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
    },
  );

  testWidgets(
    'the editor\'s Folder picker groups places by owner, and an item stays with its owner',
    (tester) async {
      await unlockOnComputer(tester);
      Future<List<String>> choices() async {
        await tester.ensureVisible(find.byKey(const ValueKey('folder-picker')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const ValueKey('folder-picker')));
        await tester.pumpAndSettle();
        final shown = find
            .descendant(
              of: find.byType(Scrollable).last,
              matching: find.byType(Text),
            )
            .evaluate()
            .map((e) => (e.widget as Text).data!)
            .toList();
        await tester.tapAt(const Offset(5, 5));
        await tester.pumpAndSettle();
        return shown;
      }

      // A new item may go to Personal's folders or the organization's.
      await tester.tap(find.text('New'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Login'));
      await tester.pumpAndSettle();
      expect(await choices(), [
        'Personal',
        'No folder',
        'Personal',
        'Contoso',
        'IT',
      ]);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();

      // The organization's item: its own shared folders only.
      await tester.tap(find.text('Office Wi-Fi'));
      await tester.pump();
      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
      await tester.pumpAndSettle();
      expect(await choices(), ['Contoso', 'IT']);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();

      // A person's own item: Personal's.
      await tester.tap(find.text('Example Mail'));
      await tester.pump();
      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
      await tester.pumpAndSettle();
      expect(await choices(), ['Personal', 'No folder', 'Personal']);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
    },
  );

  testWidgets(
    'an item favorites from its page, a login says how strong its password is and when it changed, opens its website, and has its version history',
    (tester) async {
      await unlockOnComputer(tester);
      await tester.tap(find.text('Example Mail'));
      await tester.pump();
      expect(find.textContaining('Password strong'), findsOneWidget);
      expect(
        find.textContaining(RegExp(r'^Changed \w+ \d+, \d{4}')),
        findsOneWidget,
      );
      expect(find.byTooltip('Open website'), findsOneWidget);
      // A favorite: the star takes it off, which needs the server.
      await tester.tap(find.byTooltip('Remove from favorites'));
      await pumpUntil(
        tester,
        find.text('You\'ve been signed out. Sign in again.'),
      );
      await tester.tap(find.text('Backup service'));
      await tester.pump();
      expect(find.byTooltip('Add to favorites'), findsOneWidget);
      // Its history is the server's to send.
      await tester.ensureVisible(find.text('Version history'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Version history'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to see its history'),
      );
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
    },
  );

  testWidgets('Health lists what to fix, and the menu has the generator', (
    tester,
  ) async {
    await unlockOnComputer(tester, size: const Size(1280, 1100));
    final menu = find.byType(Sidebar);
    await tester.tap(find.descendant(of: menu, matching: find.text('Health')));
    await tester.pump();
    // The sample's passwords are strong, each used once.
    expect(find.text('Nothing here'), findsOneWidget);
    await tester.tap(
      find.descendant(of: menu, matching: find.text('Generator')),
    );
    await tester.pumpAndSettle();
    expect(find.text('Copy password'), findsOneWidget);
    await tester.tap(find.text('Copy password'));
    await tester.pump();
    // Copied through the core, which clears it in 30 seconds; a computer
    // with no clipboard (CI's Linux, with no display) says it couldn't.
    expect(
      find.textContaining(
        Platform.isLinux
            ? RegExp('clears in 30 seconds|clipboard')
            : RegExp('It clears in 30 seconds'),
      ),
      findsWidgets,
    );
    await tester.tap(find.byTooltip('Close'));
    await tester.pumpAndSettle();
  });

  testWidgets(
    'a login shows its website\'s icon, kept on this device, until Settings turns icons off',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      final dir = openSample();
      // As the core keeps one it fetched (core/src/icons.rs), at the path
      // it gives.
      final sep = Platform.pathSeparator;
      Directory('$dir${sep}icons').createSync();
      final kept = '$dir${sep}icons${sep}mail.example.test.png';
      File('linux/icons/com.passbundle.app.png').copySync(kept);
      showIcons(true);
      await tester.pumpWidget(const PassBundleApp());
      // Read in real time, as the app reads it: a read left waiting in the
      // test's fake time would keep the file open, and Windows doesn't
      // delete an open file.
      final context = tester.element(find.byType(MaterialApp));
      await tester.runAsync(
        () => precacheImage(FileImage(File(kept)), context),
      );
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Example Mail'));
      await pumpUntil(tester, find.byType(Image));
      expect(find.byType(Image), findsWidgets);
      await tester.tap(find.byTooltip('Settings'));
      await tester.pump();
      await pumpUntil(tester, find.byKey(const ValueKey('show-icons')));
      await tester.tap(find.byKey(const ValueKey('show-icons')));
      await tester.pump();
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      expect(find.byType(Image), findsNothing);
      // Turning them off forgets them.
      final icons = Directory('$dir/icons');
      String? why;
      if (icons.existsSync()) {
        why = 'still kept: ${icons.listSync().map((e) => e.path).join(', ')}';
        try {
          icons.deleteSync(recursive: true);
        } on FileSystemException catch (e) {
          why = '$why; deleting it: ${e.osError ?? e.message}';
        }
      }
      expect(why, isNull);
    },
  );

  Future<void> unlockOnPhone(WidgetTester tester, Size size) async {
    setSize(tester, size);
    openSample();
    await tester.pumpWidget(const PassBundleApp());
    await tester.enterText(find.byType(TextField), samplePassword);
    await tester.tap(find.text('Unlock'));
    await pumpUntil(tester, find.text('Logins'));
    // The background sync finishes (offline: no server behind the sample).
    await pumpUntil(
      tester,
      find.textContaining('Sign in again to get changes'),
    );
  }

  testWidgets(
    'on a phone-width window, the vault starts with its kinds, an item opens on its own page, and Settings is a tab',
    (tester) async {
      await unlockOnPhone(tester, const Size(390, 844));
      // Each kind with how many there are; favorites under them.
      expect(find.text('Favorites'), findsOneWidget);
      await tester.tap(find.text('Logins'));
      await tester.pumpAndSettle();
      for (final login in ['Backup service', 'Example Mail', 'Office Wi-Fi']) {
        expect(find.text(login), findsOneWidget);
      }
      expect(find.text('Gate code'), findsNothing);

      await tester.tap(find.text('Office Wi-Fi'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Login in Contoso › IT'), findsOneWidget);
      expect(find.byTooltip('Copy password'), findsOneWidget);
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Back to all items'));
      await tester.pumpAndSettle();
      expect(find.text('Favorites'), findsOneWidget);

      await tester.tap(find.byTooltip('Settings'));
      await tester.pump();
      await pumpUntil(tester, find.text('Lock after'));
      expect(find.text('Theme'), findsOneWidget);
      await tester.tap(find.byTooltip('Items'));
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('Lock'));
      await tester.pump();
      expect(find.text('Unlock PassBundle'), findsOneWidget);
    },
  );

  testWidgets(
    'on a phone, the folders are Personal\'s, then each organization\'s shared folders, the header names the one shown, and search finds items',
    (tester) async {
      await unlockOnPhone(tester, const Size(390, 844));

      // The sample's folder is called Personal too: the owner comes first,
      // its folder after it.
      Future<void> choose(String place, {bool last = false}) async {
        // Below the kinds and favorites: the vault's list, not the search's
        // field, scrolled past the folders.
        await tester.scrollUntilVisible(
          find.text('All items'),
          100,
          scrollable: find.byType(Scrollable).last,
        );
        await tester.tap(last ? find.text(place).last : find.text(place).first);
        await tester.pumpAndSettle();
      }

      Future<void> back() async {
        await tester.tap(find.byTooltip('Back to all items'));
        await tester.pumpAndSettle();
      }

      // Personal: everything of the person's own.
      await choose('Personal');
      expect(find.text('Example Mail'), findsOneWidget);
      expect(find.text('Gate code'), findsOneWidget);
      expect(find.text('Office Wi-Fi'), findsNothing);
      expect(find.byTooltip('More for Personal'), findsNothing);
      await back();

      // Its folder.
      await choose('Personal', last: true);
      expect(find.text('Example Mail'), findsOneWidget);
      expect(find.text('Gate code'), findsNothing);
      // Its Rename and Delete are in the header while it's shown.
      expect(find.byTooltip('More for Personal'), findsOneWidget);
      await back();

      // Personal's items in no folder.
      await choose('No folder');
      expect(find.text('Gate code'), findsOneWidget);
      expect(find.text('Example Mail'), findsNothing);
      await back();

      // The organization, and its shared folder.
      await choose('Contoso');
      expect(find.text('Contoso'), findsOneWidget);
      expect(find.text('Office Wi-Fi'), findsOneWidget);
      expect(find.text('Example Mail'), findsNothing);
      await back();
      await choose('IT');
      expect(find.text('IT'), findsOneWidget);
      expect(find.text('Office Wi-Fi'), findsOneWidget);
      await back();

      await tester.enterText(find.byType(TextField), 'gate');
      await tester.pumpAndSettle();
      expect(find.text('Gate code'), findsOneWidget);
      expect(find.text('Logins'), findsNothing);
      await tester.tap(find.byTooltip('Clear the search'));
      await tester.pumpAndSettle();
      expect(find.text('Logins'), findsOneWidget);

      await tester.tap(find.byTooltip('New folder'));
      await tester.pumpAndSettle();
      expect(find.text('New folder'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
    },
  );

  testWidgets(
    'on a phone, the Generator is a tab, and its Copy goes through the phone\'s own code',
    (tester) async {
      final phone = PhoneStandIn(tester, status: 'setUp');
      await unlockOnPhone(tester, const Size(390, 844));
      await tester.tap(find.byTooltip('Generator'));
      await tester.pumpAndSettle();
      final made = tester
          .widget<SelectableText>(find.byKey(const Key('generated')))
          .data!;
      expect(made, hasLength(20));
      await tester.tap(find.text('Copy password'));
      await tester.pump();
      // The value never passes through here: the phone's code takes it from the core.
      expect(phone.copied, [
        {'id': '', 'what': 'generated'},
      ]);
      expect(find.textContaining('It clears in 30 seconds'), findsOneWidget);
      await tester.tap(find.byTooltip('Items'));
      await tester.pumpAndSettle();
      expect(find.text('Logins'), findsOneWidget);
    },
  );

  // Nothing overflows on a narrow phone with large text: a layout that
  // doesn't fit fails the test.
  for (final size in [const Size(360, 780), const Size(320, 640)]) {
    testWidgets(
      'at ${size.width.toInt()} wide with the text 1.3 times its size, every phone screen fits',
      (tester) async {
        tester.platformDispatcher.textScaleFactorTestValue = 1.3;
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
        // A phone's words, which are longer in places ("Lock when the phone
        // locks"), with biometrics already on, so nothing is offered.
        PhoneStandIn(tester, status: 'setUp');
        await unlockOnPhone(tester, size);
        await tester.tap(find.text('SSH keys'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Build server'));
        await tester.pumpAndSettle();
        await tester.pageBack();
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Back to all items'));
        await tester.pumpAndSettle();

        await tester.enterText(find.byType(TextField), 'backup');
        await tester.pumpAndSettle();
        await tester.tap(find.text('Backup service'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Edit'));
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Generate a password'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Passphrase'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Cancel').last);
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Cancel'));
        await tester.pumpAndSettle();
        await tester.pageBack();
        await tester.pumpAndSettle();

        // New item's picker, and the editor for each kind and a template.
        for (final kind in [
          'Card',
          'Identity',
          'Secure note',
          'Login',
          'Domain',
        ]) {
          await tester.tap(find.byTooltip('New item'));
          await tester.pumpAndSettle();
          if (kind == 'Domain') {
            await tester.tap(find.text('More categories'));
            await tester.pumpAndSettle();
            await tester.scrollUntilVisible(
              find.text(kind),
              100,
              scrollable: find
                  .descendant(
                    of: find.byType(NewItemPicker),
                    matching: find.byType(Scrollable),
                  )
                  .first,
            );
          }
          await tester.ensureVisible(find.text(kind));
          await tester.pumpAndSettle();
          await tester.tap(find.text(kind));
          await tester.pumpAndSettle();
          await tester.drag(
            find
                .descendant(
                  of: find.byType(ItemEditor),
                  matching: find.byType(Scrollable),
                )
                .first,
            const Offset(0, -2000),
          );
          await tester.pumpAndSettle();
          await tester.tap(find.byTooltip('Cancel'));
          await tester.pumpAndSettle();
        }

        await tester.tap(find.byTooltip('Settings'));
        await tester.pump();
        await pumpUntil(tester, find.text('Lock after'));
        await tester.ensureVisible(find.text('Lock after'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Lock after'));
        await tester.pumpAndSettle();
        expect(find.text('Away from PassBundle'), findsOneWidget);
        await tester.tap(find.text('Cancel'));
        await tester.pumpAndSettle();
        await tester.ensureVisible(find.text('Theme'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Theme'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Dark'));
        await tester.pumpAndSettle();
        expect(find.text('Dark'), findsOneWidget);

        // The Generator tab, both kinds.
        await tester.tap(find.byTooltip('Generator'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Passphrase'));
        await tester.pumpAndSettle();
        await tester.drag(find.byType(Scrollable).last, const Offset(0, -1000));
        await tester.pumpAndSettle();
      },
    );
  }
}
