// Where an organization keeps items out of Personal (upstream's "Remove
// individual vault", policy 5), a new item goes where the core says
// (core/src/app.rs, `new_item_places`): the editor offers no Personal, only
// the shared folders the person may add to, and starts in one; with none,
// New says so in the import's words and opens no editor. The person's own
// items have no Edit, and say why; a new password for one, from Android's
// autofill, goes in a new login. Its owners and admins aren't bound.
// Against the real core, with the sample vault's kept sync carrying the
// policy, as the server sends it.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/editor.dart' show ItemEditor;
import 'package:passbundle/main.dart';
import 'package:passbundle/vault.dart';

import 'launch_fixes_test.dart' show computer, phone, setSize;
import 'support.dart';

/// The sample's organization, and its shared folder IT.
const contoso = '8f9a7c1e-3b2d-4e5f-9a8b-7c6d5e4f3a2b';
const it = '2c3d4e5f-6071-4829-9ab0-c1d2e3f40516';

/// The sample's Example Mail, one of the person's own.
const mail = '3d4e5f60-7182-4930-abc1-d2e3f4051627';

/// Contoso's "Remove individual vault", on.
const keepsItemsOut = <String, dynamic>{
  'id': 'p-keeps-out',
  'organizationId': contoso,
  'type': 5,
  'enabled': true,
  'data': null,
  'object': 'policy',
};

/// The sample's profile with the person a member of Contoso (upstream's
/// type 2), not its owner, or an admin (1), as the server sends it.
Map<String, dynamic> roleInContoso(int type) {
  final sync =
      jsonDecode(File('test/sample/sync.json').readAsStringSync())
          as Map<String, dynamic>;
  final organizations = ((sync['profile'] as Map)['organizations'] as List)
      .cast<Map<String, dynamic>>();
  return {
    'organizations': [
      for (final o in organizations) {...o, 'type': type},
    ],
  };
}

/// The sample, unlocked at [size], with Contoso keeping items out of
/// Personal and the person in it as [role]; [changed] as `openSample` takes it.
Future<void> unlock(
  WidgetTester tester, {
  required Size size,
  int role = 2,
  Map<String, Map<String, dynamic>> changed = const {},
}) async {
  setSize(tester, size);
  openSample(
    policies: [keepsItemsOut],
    profile: roleInContoso(role),
    changed: changed,
  );
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text('Example Mail'));
  // The sync on opening is done (there's no server), so the core is free.
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
  await tester.pumpAndSettle();
}

/// New item, then a login: its editor.
Future<void> newLogin(WidgetTester tester) async {
  await tester.tap(find.byTooltip('New item'));
  await tester.pumpAndSettle();
  await tester.tap(find.text('Website or app'));
  await tester.pumpAndSettle();
  expect(find.byType(ItemEditor), findsOneWidget);
}

/// The editor's Folder picker, scrolled to.
Future<Finder> folderPicker(WidgetTester tester) async {
  final picker = find.byKey(const ValueKey('folder-picker'));
  await tester.dragUntilVisible(
    picker,
    find
        .descendant(
          of: find.byType(ItemEditor),
          matching: find.byType(Scrollable),
        )
        .first,
    const Offset(0, -200),
  );
  await tester.ensureVisible(picker);
  await tester.pumpAndSettle();
  return picker;
}

/// Where the editor's Folder picker puts the item: `personal:` and a folder,
/// or `org:`, an organization, and a shared folder.
Future<String?> picked(WidgetTester tester) async => tester
    .widget<DropdownButtonFormField<String>>(await folderPicker(tester))
    .initialValue;

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'a member it binds starts a new item in a shared folder, and the Folder picker offers no Personal',
    (tester) async {
      await unlock(tester, size: phone);
      await newLogin(tester);
      expect(await picked(tester), 'org:$contoso:$it');
      expect(
        find.textContaining('Contoso keeps items out of Personal.'),
        findsOneWidget,
      );
      // Only Contoso's IT: not Personal, nor its folders, nor No folder.
      await tester.tap(await folderPicker(tester));
      await tester.pumpAndSettle();
      expect(find.text('IT'), findsWidgets);
      expect(find.text('Contoso'), findsOneWidget);
      expect(find.text('Personal'), findsNothing);
      expect(find.text('No folder'), findsNothing);
    },
  );

  testWidgets(
    'on a computer, a new item made while Personal is shown starts in the shared folder too',
    (tester) async {
      await unlock(tester, size: const Size(1280, 1100));
      final personal = find
          .descendant(of: find.byType(Sidebar), matching: find.text('Personal'))
          .first;
      await tester.tap(personal);
      await tester.pumpAndSettle();
      await newLogin(tester);
      expect(await picked(tester), 'org:$contoso:$it');
    },
  );

  testWidgets(
    'with no shared folder the member can add to, New says so in the import\'s words and opens no editor',
    (tester) async {
      // IT, which they may only see.
      await unlock(
        tester,
        size: phone,
        changed: {
          it: {'readOnly': true},
        },
      );
      await tester.tap(find.byTooltip('New item'));
      await tester.pump();
      expect(
        find.text(
          'Contoso keeps items out of Personal, and you have no shared folder you can add to. Ask an admin for one, then try again.',
        ),
        findsOneWidget,
      );
      await tester.pumpAndSettle();
      expect(find.text('What are you saving?'), findsNothing);
      expect(find.byType(ItemEditor), findsNothing);
    },
  );

  testWidgets(
    'their own item has no Edit and says why, and Contoso\'s still does',
    (tester) async {
      await unlock(tester, size: computer);
      final edit = find.widgetWithText(OutlinedButton, 'Edit');
      await tester.tap(find.widgetWithText(ItemRow, 'Example Mail'));
      await tester.pumpAndSettle();
      expect(edit, findsNothing);
      expect(
        tester.widget<Text>(find.byKey(const ValueKey('kept-out'))).data,
        'Contoso keeps items out of Personal, so this item can\'t be changed. You can move it to a shared folder in the web vault.',
      );
      await tester.tap(find.widgetWithText(ItemRow, 'Office Wi-Fi'));
      await tester.pumpAndSettle();
      expect(edit, findsOneWidget);
      expect(find.byKey(const ValueKey('kept-out')), findsNothing);
    },
  );

  testWidgets(
    'a new password autofill offered to save for one of their own logins goes in a new login, in the shared folder',
    (tester) async {
      await unlock(tester, size: computer);
      final vault = tester.state<VaultScreenState>(find.byType(VaultScreen));
      // As Android's "Save to PassBundle" hands it over, with the login it
      // found for the site (PassBundleAutofill.kt, takeSave).
      unawaited(
        vault.edit(mail, {
          'name': 'mail.example.test',
          'username': 'ana@mail.example.test',
          'password': 'not-a-real-password-new',
          'uri': 'https://mail.example.test',
          'id': mail,
        }),
      );
      await tester.pumpAndSettle();
      expect(find.text('New login'), findsOneWidget);
      expect(find.text('ana@mail.example.test'), findsOneWidget);
      expect(await picked(tester), 'org:$contoso:$it');
    },
  );

  for (final (who, role) in [('an owner', 0), ('an admin', 1)]) {
    testWidgets(
      'for $who, whom it doesn\'t bind, a new item starts in Personal and their own items change',
      (tester) async {
        await unlock(tester, size: phone, role: role);
        await newLogin(tester);
        expect(await picked(tester), 'personal:');
        expect(
          find.textContaining('keeps items out of Personal'),
          findsNothing,
        );
        // Personal and its folders, and IT.
        await tester.tap(await folderPicker(tester));
        await tester.pumpAndSettle();
        expect(find.text('Personal'), findsWidgets);
        expect(find.text('IT'), findsWidgets);
        // Picked as it was: nothing changed, so Cancel just closes it.
        await tester.tap(find.text('No folder').last);
        await tester.pumpAndSettle();
        expect(await picked(tester), 'personal:');
        await tester.tap(find.byTooltip('Cancel'));
        await tester.pumpAndSettle();
        expect(find.byType(ItemEditor), findsNothing);

        // A new password for their own login opens that login.
        final vault = tester.state<VaultScreenState>(
          find.byType(VaultScreen),
        );
        unawaited(
          vault.edit(mail, {
            'name': 'mail.example.test',
            'username': 'ana@mail.example.test',
            'password': 'not-a-real-password-new',
            'uri': 'https://mail.example.test',
            'id': mail,
          }),
        );
        await tester.pumpAndSettle();
        expect(find.text('Edit login'), findsOneWidget);
      },
    );
  }
}
