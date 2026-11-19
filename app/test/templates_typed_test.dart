// Templates in the app (core/src/templates.rs; the meta repo's DESIGN.md,
// "Item templates"): New item's Create template, and Save as template on a
// login, each opening a new template; the editor's fields with a
// template's kinds (a date with its calendar, a website checked as it's
// typed, a warning that never stops a save); a template's expiration date
// field and Expires kept the same, both ways; a one-time code field's
// current code in the item's view; and Item templates in the menu, with
// templates out of All items. Against the real core, the sample vault's
// items given a template's fields by the core's test hook, as the web vault
// would leave them; the tests' vault has no server, so what Save would
// send is read before it's sent.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/editor.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/new_item.dart';
import 'package:passbundle/vault.dart';

import 'support.dart';

/// The sample's own login (the core's vectors).
const exampleMail = '3d4e5f60-7182-4930-abc1-d2e3f4051627';

/// The sample's organization's login, without a one-time code of its own.
const officeWifi = '4e5f6071-8293-4a41-bcd2-e3f405162738';

/// A key for one-time codes, made up (RFC 4226's example key).
const codeKey = 'GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ';

void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// A custom field as the web vault saves one: upstream's type, 0 text, 1
/// hidden.
Map<String, dynamic> field(String name, String value, [int type = 0]) => {
  'name': name,
  'value': value,
  'type': type,
};

/// The sample vault, unlocked on a computer, with [fields] added to its
/// items (by id) once its offline sync has given up.
Future<void> unlock(
  WidgetTester tester, {
  Map<String, List<Map<String, dynamic>>> fields = const {},
}) async {
  setSize(tester, const Size(1280, 1100));
  openSample();
  Core().testFields(fields);
  addTearDown(() => Core().testFields(const {}));
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text('Example Mail'));
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
}

/// Scrolls the editor until [finder] shows: to it, where the list has
/// built it, else down until it's built.
Future<void> reveal(WidgetTester tester, Finder finder) async {
  if (finder.evaluate().isEmpty) {
    await tester.scrollUntilVisible(
      finder,
      200,
      scrollable: find
          .descendant(
            of: find.byType(ItemEditor),
            matching: find.byType(Scrollable),
          )
          .first,
    );
  }
  await tester.ensureVisible(finder);
  await tester.pumpAndSettle();
}

/// The editor's Add a field, with a kind and a name.
Future<void> addField(WidgetTester tester, String kind, String name) async {
  await reveal(tester, find.byTooltip('Add a field'));
  await tester.tap(find.byTooltip('Add a field'));
  await tester.pumpAndSettle();
  await tester.tap(find.text(kind).last);
  await tester.pumpAndSettle();
  final dialog = find.byType(AlertDialog);
  await tester.enterText(
    find.descendant(of: dialog, matching: find.byType(TextField)),
    name,
  );
  await tester.pump();
  await tester.tap(find.descendant(of: dialog, matching: find.text('Save')));
  await tester.pumpAndSettle();
}

/// What the editor's Save sends the core, once it's pressed.
Future<Map<String, dynamic>> save(WidgetTester tester) async {
  Map<String, dynamic>? sent;
  ItemEditor.debugSaving = (draft) => sent = draft;
  addTearDown(() => ItemEditor.debugSaving = null);
  await tester.tap(find.text('Save').last);
  await tester.pump();
  expect(sent, isNotNull);
  // There's no server here: it isn't saved, and the editor says why.
  await pumpUntil(
    tester,
    find.descendant(
      of: find.byType(ItemEditor),
      matching: find.byWidgetPredicate(
        (w) =>
            w is Text &&
            (w.data?.contains('Sign in again') == true ||
                w.data?.contains('Not saved') == true),
      ),
    ),
  );
  return sent!;
}

/// Closes the editor without saving.
Future<void> discard(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Cancel'));
  await tester.pumpAndSettle();
  if (find.text('Discard').evaluate().isNotEmpty) {
    await tester.tap(find.text('Discard'));
    await tester.pumpAndSettle();
  }
}

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'New item\'s Create template asks what its items will be, and the template\'s fields take kinds and an expiration date',
    (tester) async {
      await unlock(tester);
      await tester.tap(find.text('New'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('More categories'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.byKey(const ValueKey('create-template')));
      await tester.tap(find.byKey(const ValueKey('create-template')));
      await tester.pumpAndSettle();
      expect(find.text('New template'), findsOneWidget);
      expect(find.text('New items from this template will be:'), findsOneWidget);
      await tester.tap(
        find.descendant(
          of: find.byType(NewItemPicker),
          matching: find.text('Logins'),
        ),
      );
      await tester.pumpAndSettle();

      // The editor makes a template: it says so.
      expect(find.byType(ItemEditor), findsOneWidget);
      expect(find.text('New template'), findsOneWidget);
      expect(
        find.textContaining('This is a template for new logins.'),
        findsOneWidget,
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Name').first,
        'Software license',
      );
      await addField(tester, 'Date', 'Renews');
      await addField(tester, 'Website', 'Vendor site');
      await addField(tester, 'One-time code', 'Console code');
      // Renews is when new items expire.
      await reveal(tester, find.byTooltip('More for Renews'));
      await tester.tap(find.byTooltip('More for Renews'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('New items expire on this date'));
      await tester.pumpAndSettle();
      expect(find.text('Date. New items expire on this date.'), findsOneWidget);
      expect(find.text('Website.'), findsOneWidget);
      expect(find.byTooltip('Pick a date for Renews'), findsOneWidget);

      final draft = await save(tester);
      expect(draft['note'], isTrue);
      expect(draft['newTemplate'], 'login');
      expect(draft['name'], 'Software license');
      expect(draft['typing'], {
        'types': {'Renews': 'date', 'Vendor site': 'url', 'Console code': 'totp'},
        'expires': 'Renews',
      });
      expect(
        [for (final f in draft['fields'] as List) (f['name'], f['type'])],
        [('Renews', 0), ('Vendor site', 0), ('Console code', 1)],
      );
      await discard(tester);
    },
  );

  testWidgets(
    'Save as template on a login opens a new template with its fields\' labels and kinds, never their values',
    (tester) async {
      await unlock(
        tester,
        fields: {
          exampleMail: [
            field('Renews', '2027-06-30'),
            field('Admin PIN', 'not-a-real-pin-4421', 1),
            field(
              'keykeeper.template',
              '{"id":"lic","name":"License","v":1,"types":{"Renews":"date"},"expires":"Renews"}',
            ),
          ],
        },
      );
      await tester.tap(find.text('Example Mail'));
      await tester.pump();
      await tester.tap(find.byTooltip('Save as template'));
      await tester.pumpAndSettle();

      expect(find.text('New template'), findsOneWidget);
      expect(
        find.textContaining('This is a template for new logins.'),
        findsOneWidget,
      );
      expect(
        find.widgetWithText(TextField, 'Example Mail'),
        findsOneWidget,
        reason: 'named as the item is',
      );
      await reveal(tester, find.widgetWithText(TextField, 'Renews'));
      expect(find.text('Date. New items expire on this date.'), findsOneWidget);
      // The login's own fields and the template's, none of their values.
      final draft = await save(tester);
      expect(draft['newTemplate'], 'login');
      expect(draft['typing'], {
        'types': {'Renews': 'date'},
        'expires': 'Renews',
      });
      expect(
        [
          for (final f in draft['fields'] as List)
            (f['name'], f['value'], f['type']),
        ],
        [
          ('Account number', '', 0),
          ('Recovery PIN', '', 1),
          ('Renews', '', 0),
          ('Admin PIN', '', 1),
        ],
      );
      for (final value in ['12345', '8642', '2027-06-30', 'not-a-real-pin-4421']) {
        expect(draft.toString(), isNot(contains(value)));
      }
      await discard(tester);
    },
  );

  testWidgets(
    'an item from a template: a date picker, a warning that never stops a save, and Expires kept with the expiration date field',
    (tester) async {
      await unlock(
        tester,
        fields: {
          exampleMail: [
            field('Renews', ''),
            field('Vendor site', 'not a site'),
            field(
              'keykeeper.template',
              '{"id":"lic","name":"License","v":1,"types":{"Renews":"date","Vendor site":"url"},"expires":"Renews"}',
            ),
          ],
        },
      );
      await tester.tap(find.text('Example Mail'));
      await tester.pump();
      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
      await tester.pumpAndSettle();

      final renews = find.widgetWithText(TextField, 'Renews');
      await reveal(tester, renews);
      expect(find.byTooltip('Pick a date for Renews'), findsOneWidget);
      expect(find.text('Also when this item expires.'), findsOneWidget);
      expect(
        find.text("This doesn't look like a web address."),
        findsOneWidget,
      );

      // Filling the date sets Expires.
      await tester.enterText(renews, '2027-06-30');
      await tester.pumpAndSettle();
      final expires = find.widgetWithText(TextField, 'Expires (optional)');
      await reveal(tester, expires);
      expect(
        tester.widget<TextField>(expires).controller!.text,
        '2027-06-30',
      );

      // Changing Expires changes the field.
      await tester.enterText(expires, '2027-09-30');
      await tester.pumpAndSettle();
      await reveal(tester, renews);
      expect(tester.widget<TextField>(renews).controller!.text, '2027-09-30');

      // The website's warning doesn't stop the save.
      final draft = await save(tester);
      expect(draft['expires'], '2027-09-30');
      expect(draft['typing'], {
        'types': {'Renews': 'date', 'Vendor site': 'url'},
        'expires': 'Renews',
      });
      expect(draft.containsKey('newTemplate'), isFalse);
      await discard(tester);
    },
  );

  testWidgets(
    'the item\'s view shows a one-time code field\'s current code, and Item templates has the templates, out of All items',
    (tester) async {
      await unlock(
        tester,
        fields: {
          officeWifi: [
            field('Console code', codeKey, 1),
            field(
              'keykeeper.template',
              '{"id":"vendor","name":"Vendor console","v":1,"types":{"Console code":"totp"}}',
            ),
          ],
          // Gate code is a template for notes.
          gateCode: [
            field(
              'keykeeper.template-definition',
              '{"id":"gate","base":"note","v":1}',
            ),
          ],
        },
      );
      final menu = find.byType(Sidebar);
      Finder entry(String label) =>
          find.descendant(of: menu, matching: find.text(label));

      // The template isn't in All items, but under Item templates.
      expect(find.text('Gate code'), findsNothing);
      expect(entry('Item templates'), findsOneWidget);
      await tester.tap(entry('Item templates'));
      await tester.pumpAndSettle();
      expect(find.text('Gate code'), findsOneWidget);
      expect(find.text('Example Mail'), findsNothing);
      await tester.tap(entry('All items'));
      await tester.pumpAndSettle();

      // The one-time code field shows its code, and copies it.
      await tester.tap(find.text('Office Wi-Fi'));
      await pumpUntil(tester, find.byTooltip('Copy console code'));
      expect(find.text(codeKey), findsNothing);
      expect(
        find.textContaining(RegExp(r'^\d{3} \d{3}$')),
        findsOneWidget,
      );
    },
  );

  test('the checks a typed field\'s warning says, as the web vault\'s', () {
    expect(typedFieldProblem('url', 'https://vendor.example.test/login'), isNull);
    expect(typedFieldProblem('url', 'vendor.example'), isNull);
    expect(
      typedFieldProblem('url', 'ftp://vendor.example'),
      "This doesn't look like a web address.",
    );
    expect(typedFieldProblem('email', 'it@contoso.example'), isNull);
    expect(
      typedFieldProblem('email', 'it@contoso'),
      "This doesn't look like an email address.",
    );
    expect(typedFieldProblem('phone', '+1 (555) 010-4477'), isNull);
    expect(
      typedFieldProblem('phone', '12'),
      "This doesn't look like a phone number.",
    );
    expect(typedFieldProblem('number', '-1,250.5'), isNull);
    expect(typedFieldProblem('number', 'about 3'), "This isn't a number.");
    expect(typedFieldProblem('date', '2027-03-31'), isNull);
    expect(
      typedFieldProblem('date', '2027-02-30'),
      'Write the date as year, month, and day, such as 2027-03-31.',
    );
    for (final kind in ['url', 'email', 'phone', 'number', 'date']) {
      expect(typedFieldProblem(kind, '  '), isNull);
    }
    expect(typedFieldProblem(null, 'anything'), isNull);
    expect(typedFieldProblem('totp', 'anything'), isNull);
  });
}
