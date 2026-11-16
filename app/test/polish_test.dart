// The apps' polish and parity gaps (the apps check's sections 2 and 3),
// against the real core: the menu's tools in their order after the folders
// and the trash, the list fading at its foot, Favorites only once something is one, an empty vault
// saying what to do first, red last steps, notices clear of the menu, the
// New picker naming what only the web vault makes and as tall as what's in
// it, a new folder going where the person says, the generator's history,
// "Ask for master password" in the editor, "Change password" on an at-risk
// login, the master password hint and its strength before signing in,
// Health's reports on a computer and a phone, the lock screen's server under
// its email, and the account's settings in the web vault from Settings.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/editor.dart' show ItemEditor;
import 'package:passbundle/generator.dart';
import 'package:passbundle/glyphs.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/places.dart';
import 'package:passbundle/theme.dart';
import 'package:passbundle/vault.dart';
import 'package:passbundle/widgets.dart';

import 'launch_fixes_test.dart'
    show StandIn, computer, edit, exampleMail, phone, setSize, signIn, unlock;
import 'support.dart';

/// A cipher as the stand-in server keeps it now.
Map<String, dynamic> kept(StandIn standIn, String id) =>
    (standIn.sync['ciphers'] as List).cast<Map<String, dynamic>>().firstWhere(
      (c) => c['id'] == id,
    );

/// Pumps until [done] says so, giving slow core calls real time.
Future<void> waitFor(WidgetTester tester, bool Function() done) async {
  final end = DateTime.now().add(const Duration(seconds: 20));
  while (!done() && DateTime.now().isBefore(end)) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
    await tester.pump();
  }
  expect(done(), isTrue);
}

Finder inMenu(Finder finder) =>
    find.descendant(of: find.byType(Sidebar), matching: finder);

void main() {
  setUpAll(loadFonts);

  group('the menu on a computer', () {
    testWidgets(
      'has its tools in order after the folders and the trash, fades at its foot, and shows Favorites only once something is one',
      (tester) async {
        await unlock(
          tester,
          const Size(1280, 1100),
          changed: {
            exampleMail: {'favorite': false},
          },
        );
        expect(inMenu(find.text('Favorites')), findsNothing);
        expect(inMenu(find.text('Trash')), findsOneWidget);
        double top(String label) =>
            tester.getTopLeft(inMenu(find.text(label))).dy;
        expect(top('Generator'), lessThan(top('Import')));
        expect(top('Import'), lessThan(top('Import from browsers')));
        expect(top('Import from browsers'), lessThan(top('Export')));
        // After a line, past the last folder and the trash, as the web
        // vault's menu has them.
        expect(top('Trash'), lessThan(top('Generator')));
        // Export goes out of PassBundle: its own glyph, not Import's.
        expect(
          inMenu(
            find.byWidgetPredicate(
              (w) => w is Glyph && w.data == Glyphs.upload,
            ),
          ),
          findsOneWidget,
        );
        expect(find.byType(ShaderMask), findsWidgets);
      },
    );

    testWidgets(
      'an empty vault says what to do first, and nothing asks to choose an item',
      (tester) async {
        final ids = [
          for (final c
              in (jsonDecode(File('test/sample/sync.json').readAsStringSync())
                      as Map<String, dynamic>)['ciphers']
                  as List)
            (c as Map)['id'] as String,
        ];
        await unlock(
          tester,
          computer,
          changed: {
            for (final id in ids) id: {'deletedDate': '2026-11-01T10:00:00.000Z'},
          },
          ready: 'Nothing saved yet',
        );
        final empty = find.byKey(const ValueKey('empty-vault'));
        expect(empty, findsOneWidget);
        expect(find.text('Choose an item'), findsNothing);
        for (final action in [
          'New item',
          'Import from browsers',
          'Import a file in the web vault',
        ]) {
          expect(
            find.descendant(of: empty, matching: find.text(action)),
            findsOneWidget,
          );
        }
        await tester.tap(find.descendant(of: empty, matching: find.text('New item')));
        await tester.pumpAndSettle();
        expect(find.text('What are you saving?'), findsOneWidget);
      },
    );

    testWidgets('moving an item to the trash has a red last step, and notices float clear of the menu', (
      tester,
    ) async {
      await unlock(tester, computer);
      await tester.tap(find.text('Backup service'));
      await tester.pump();
      await tester.tap(find.byTooltip('Move to trash'));
      await tester.pumpAndSettle();
      final question = find.text('Move Backup service to the trash?');
      expect(question, findsOneWidget);
      final red = tester.widget<FilledButton>(
        find.widgetWithText(FilledButton, 'Move to trash'),
      );
      expect(
        red.style?.backgroundColor?.resolve({}),
        Theme.of(tester.element(question)).colorScheme.error,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      // A notice (copying, here) sits over the list, not the menu's foot.
      await tester.tap(find.byTooltip('Copy username'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 400));
      final said = find.descendant(
        of: find.byType(SnackBar),
        matching: find.byType(Text),
      );
      expect(said, findsWidgets);
      expect(
        tester.getTopLeft(said.first).dx,
        greaterThan(12 + sidebarWidth),
      );
    });
  });

  testWidgets(
    'the New picker is as tall as what\'s in it, gives templates their own glyphs, and names what only the web vault makes',
    (tester) async {
      await unlock(tester, computer);
      await tester.tap(find.byTooltip('New item'));
      await tester.pumpAndSettle();
      expect(find.text('What are you saving?'), findsOneWidget);
      // The dialog's own card, not the room around it.
      final card = find
          .descendant(of: find.byType(Dialog), matching: find.byType(Material))
          .first;
      expect(tester.getSize(card).height, lessThan(600));
      await tester.tap(find.text('More categories'));
      await tester.pumpAndSettle();
      // A database isn't drawn as a server.
      expect(
        find.byWidgetPredicate(
          (w) => w is JewelTile && w.jewel.glyph == Glyphs.database,
        ),
        findsOneWidget,
      );
      final elsewhere = find.text('In the web vault');
      await tester.ensureVisible(elsewhere);
      await tester.pumpAndSettle();
      for (final kind in ['SSH key', 'Bank account', 'Driver\'s license', 'Passport']) {
        expect(find.text(kind), findsOneWidget);
      }
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
      expect(find.text('What are you saving?'), findsNothing);
    },
  );

  testWidgets(
    'a new folder goes in Personal or an organization where the person may make one, at the top or inside another',
    (tester) async {
      setSize(tester, computer);
      final places = Places({
        'folders': [
          {'id': 'f1', 'name': 'Work', 'label': 'Work', 'depth': 0, 'parentId': null},
        ],
        'organizations': [
          {
            'id': 'o1',
            'name': 'Contoso',
            'canMakeFolders': true,
            'collections': [
              {'id': 'c1', 'name': 'IT', 'label': 'IT', 'depth': 0, 'parentId': null},
            ],
          },
          {
            'id': 'o2',
            'name': 'Fabrikam',
            'canMakeFolders': false,
            'collections': const [],
          },
        ],
      });
      NewFolder? made;
      await tester.pumpWidget(
        MaterialApp(
          theme: passBundleTheme(Brightness.light),
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () async => made = await askNewFolder(context, places),
                child: const Text('Open'),
              ),
            ),
          ),
        ),
      );
      Future<void> make(String name, String inside) async {
        await tester.tap(find.text('Open'));
        await tester.pumpAndSettle();
        await tester.enterText(find.widgetWithText(TextField, 'Name'), name);
        await tester.tap(find.byKey(const ValueKey('new-folder-in')));
        await tester.pumpAndSettle();
        // Fabrikam doesn't let this person make folders: it isn't offered.
        expect(find.text('Fabrikam'), findsNothing);
        await tester.tap(find.text(inside).last);
        await tester.pumpAndSettle();
        await tester.tap(find.text('Save'));
        await tester.pumpAndSettle();
      }

      await make('Bills', 'Personal');
      expect(made, (name: 'Bills', organization: null));
      await make('Clients', 'Work');
      expect(made, (name: 'Work/Clients', organization: null));
      await make('Servers', 'IT');
      expect(made, (name: 'IT/Servers', organization: 'o1'));
      await make('Everyone', 'Contoso');
      expect(made, (name: 'Everyone', organization: 'o1'));
    },
  );

  testWidgets(
    'the generator keeps what was copied in its history until it\'s cleared, and colors its digits and symbols',
    (tester) async {
      await unlock(tester, computer);
      expect(Core().generatorHistory(), isEmpty);
      await tester.tap(await menuEntry(tester, 'Generator'));
      await tester.pumpAndSettle();
      final made = generatedText(tester);
      final shown = tester.widget<GeneratedText>(
        find.ancestor(
          of: find.byKey(const Key('generated')),
          matching: find.byType(GeneratedText),
        ),
      );
      expect(shown.text, made);
      await tester.tap(find.text('Copy password'));
      await tester.pump();
      expect(
        Core().generatorHistory().map((h) => h['value']).toList(),
        [made],
      );
      await tester.tap(find.byTooltip('Generator history'));
      await tester.pumpAndSettle();
      final history = find.byType(Dialog).last;
      expect(
        find.descendant(
          of: history,
          matching: find.byWidgetPredicate(
            (w) => w is GeneratedText && w.text == made,
          ),
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Clear history'));
      await tester.pump();
      expect(find.text('Nothing yet.'), findsOneWidget);
      expect(Core().generatorHistory(), isEmpty);
    },
  );

  group('with a server', () {
    testWidgets(
      'the editor turns "Ask for master password" on and off, and the item asks once it\'s on',
      (tester) async {
        final standIn = await StandIn.start(tester);
        await signIn(tester, standIn);
        await edit(tester, 'Example Mail');
        final ask = find.byKey(const ValueKey('ask-for-master-password'));
        // At the editor's foot, past what its list has built yet.
        Future<void> down() async {
          await tester.scrollUntilVisible(
            ask,
            200,
            scrollable: find
                .descendant(
                  of: find.byType(ItemEditor),
                  matching: find.byType(Scrollable),
                )
                .first,
          );
          await tester.pumpAndSettle();
        }

        await down();
        expect(tester.widget<SwitchListTile>(ask).value, isFalse);
        await tester.tap(ask);
        await tester.pump();
        await tester.tap(find.text('Save'));
        await waitFor(tester, () => kept(standIn, exampleMail)['reprompt'] == 1);
        await pumpUntil(
          tester,
          find.text('This item asks for your master password.'),
        );

        // Given, the editor shows it on, and turns it off.
        await tester.enterText(
          find.widgetWithText(TextField, 'Master password'),
          samplePassword,
        );
        await tester.tap(find.text('Continue'));
        await pumpUntil(tester, find.widgetWithText(OutlinedButton, 'Edit'));
        await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
        await tester.pumpAndSettle();
        await down();
        expect(tester.widget<SwitchListTile>(ask).value, isTrue);
        await tester.tap(ask);
        await tester.pump();
        await tester.tap(find.text('Save'));
        await waitFor(tester, () => kept(standIn, exampleMail)['reprompt'] == 0);
      },
    );

    testWidgets(
      'a login with a weak password offers Change password, which opens the site\'s own page',
      (tester) async {
        final standIn = await StandIn.start(tester);
        await signIn(tester, standIn);
        expect(find.byKey(const ValueKey('change-password')), findsNothing);
        await edit(tester, 'Example Mail');
        await tester.enterText(
          find.widgetWithText(TextField, 'Password'),
          'password1',
        );
        await tester.enterText(
          find.widgetWithText(TextField, 'Website').first,
          'https://www.23andme.com/',
        );
        final before = kept(standIn, exampleMail)['revisionDate'];
        await tester.tap(find.text('Save'));
        await waitFor(
          tester,
          () => kept(standIn, exampleMail)['revisionDate'] != before,
        );
        await pumpUntil(tester, find.byKey(const ValueKey('change-password')));
        // Apple's list has the site's page, found on this device.
        final page = await tester.runAsync(
          () => Core.changePasswordPage(exampleMail),
        );
        expect(page!.ok, isTrue);
        expect(page.body['url'], 'https://you.23andme.com/user/edit/password/');
      },
    );

    testWidgets(
      'the sign-in screen gets the master password hint for the email typed, from the server shown',
      (tester) async {
        final standIn = await StandIn.start(tester);
        standIn.hold('/api/accounts/password-hint').complete((200, null));
        openEmpty();
        setSize(tester, computer);
        await tester.pumpWidget(const PassBundleApp());
        await tester.tap(find.text('Self-hosted?'));
        await tester.pump();
        await tester.enterText(
          find.widgetWithText(TextField, 'Server'),
          standIn.url,
        );
        // No email yet: it says so, under the email, and asks nothing.
        await tester.tap(find.text('Get your master password hint'));
        await tester.pump();
        expect(
          find.text('Type your email first, then ask for your hint.'),
          findsOneWidget,
        );
        expect(standIn.asked, isNot(contains('POST /api/accounts/password-hint')));
        await tester.enterText(
          find.widgetWithText(TextField, 'Email'),
          'ana.lopez@example.test',
        );
        await tester.tap(find.text('Get your master password hint'));
        await pumpUntil(
          tester,
          find.textContaining('its server emailed the hint there'),
        );
        expect(find.text('Master password hint'), findsOneWidget);
        expect(standIn.asked, contains('POST /api/accounts/password-hint'));
      },
    );
  });

  testWidgets(
    'making an account shows how strong the master password is as it\'s typed',
    (tester) async {
      openEmpty();
      setSize(tester, computer);
      await tester.pumpWidget(const PassBundleApp());
      await tester.tap(find.text('Create account'));
      await tester.pump();
      final meter = find.byKey(const ValueKey('password-strength'));
      expect(meter, findsNothing);
      final field = find.widgetWithText(TextField, 'Master password');
      await tester.enterText(field, 'password');
      await tester.pump();
      expect(find.descendant(of: meter, matching: find.text('Weak')), findsOneWidget);
      await tester.enterText(field, 'kq9#Vz!m2P@xL7&r-Wt4');
      await tester.pump();
      expect(
        find.descendant(of: meter, matching: find.text('Strong')),
        findsOneWidget,
      );
      await tester.enterText(field, '');
      await tester.pump();
      expect(meter, findsNothing);
    },
  );

  testWidgets(
    'Health sums up each report, which narrows its list, on a computer and a phone',
    (tester) async {
      await unlock(
        tester,
        phone,
        expiring: {
          exampleMail: {'expires': isoInDays(5)},
        },
        ready: 'Logins',
      );
      final card = find.byKey(const ValueKey('health'));
      await tester.ensureVisible(card);
      await tester.pumpAndSettle();
      await pumpUntil(
        tester,
        find.descendant(of: card, matching: find.text('1 to fix')),
      );
      await tester.tap(card);
      await tester.pumpAndSettle();
      final reports = find.byKey(const ValueKey('health-reports'));
      expect(
        find.descendant(of: reports, matching: find.text('Expiring items')),
        findsOneWidget,
      );
      expect(
        find.descendant(of: reports, matching: find.text('Weak passwords')),
        findsNothing,
      );
      await tester.tap(find.text('Expiring items'));
      await tester.pumpAndSettle();
      expect(find.text('Example Mail'), findsOneWidget);
      // The phone's own categories scroll sideways, in one row.
      await tester.tap(find.byTooltip('Back to all items'));
      await tester.pumpAndSettle();
      final kinds = find.byKey(const ValueKey('kinds'));
      expect(
        tester.widget<SingleChildScrollView>(kinds).scrollDirection,
        Axis.horizontal,
      );
    },
  );

  testWidgets(
    'the lock screen says the server under the email, and Settings opens the account\'s own settings in the web vault',
    (tester) async {
      setSize(tester, computer);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      final host = find.text('vault.example.test');
      expect(host, findsOneWidget);
      expect(
        tester.getTopLeft(host).dy,
        lessThan(
          tester.getTopLeft(find.widgetWithText(TextField, 'Master password')).dy,
        ),
      );
      expect(
        tester.getTopLeft(find.text('ana.lopez@example.test')).dy,
        lessThan(tester.getTopLeft(host).dy),
      );

      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Example Mail'));
      await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
      await tester.tap(find.byTooltip('Settings'));
      await tester.pump();
      await pumpUntil(tester, find.text('Lock after'));
      final row = find.byKey(const ValueKey('web-account-settings'));
      expect(row, findsOneWidget);
      // The account comes first.
      expect(
        tester.getTopLeft(row).dy,
        lessThan(tester.getTopLeft(find.text('Lock after')).dy),
      );
    },
  );
}
