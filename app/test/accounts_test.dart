// Several accounts side by side (accounts.dart; DESIGN.md in the meta repo,
// "Several accounts in the apps"), against the real core (PB_CORE_LIB): the
// account menu at the foot of the computer's menu and in a phone's Settings
// switches between a work account and a personal one, each unlocked and
// locked on its own and showing only its own items; adding one goes to the
// sign-in screen and back; signing out of one leaves the other; at five,
// adding waits; and an install from before several accounts is carried over.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/settings.dart';
import 'package:passbundle/unlock_offer.dart';

import 'support.dart';

const work = 'ana.lopez@example.test';
const home = 'ana@home.example.test';

void wideWindow(WidgetTester tester) {
  tester.view.physicalSize = const Size(1280, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

void phoneWindow(WidgetTester tester) {
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// Each account's state, by its id, as the core has it.
Map<String, String> states() => {
  for (final a in (Core().accounts()['accounts'] as List).cast<Map>())
    a['id'] as String: a['state'] as String,
};

Future<void> unlock(WidgetTester tester, String shows) async {
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await opened(tester, shows);
}

/// A vault showing [shows], past the sync on opening (there's no server
/// behind the sample): each time a vault shows, unlocked just now or
/// switched to.
Future<void> opened(WidgetTester tester, String shows) async {
  await pumpUntil(tester, find.text(shows));
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
  await tester.pumpAndSettle();
}

/// The account menu, from the email at the foot of the menu.
Future<void> openMenu(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Accounts'));
  await tester.pumpAndSettle();
}

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'the account menu switches between two accounts, each unlocked and locked on its own, each with only its own items',
    (tester) async {
      wideWindow(tester);
      final [w, p] = openAccounts();
      await tester.pumpWidget(const PassBundleApp());
      // The one shown last, locked, with the way to the other.
      expect(find.text('Unlock PassBundle'), findsOneWidget);
      expect(find.text(work), findsOneWidget);
      expect(find.text('Switch account'), findsOneWidget);

      await unlock(tester, 'Example Mail');
      expect(find.text('Gate code'), findsNothing);
      expect(states(), {w: 'unlocked', p: 'locked'});

      // The menu lists both, the one shown marked, the other locked.
      await openMenu(tester);
      expect(find.text(home), findsOneWidget);
      expect(find.text('vault.home.example.test · Locked'), findsOneWidget);
      expect(find.text('Add account'), findsOneWidget);
      expect(find.text('Sign out of this account'), findsOneWidget);
      await tester.tap(find.text(home));
      await tester.pumpAndSettle();
      // The personal account, locked; the work one stays open.
      expect(find.text('Unlock PassBundle'), findsOneWidget);
      expect(find.text(home), findsOneWidget);
      expect(find.text('Example Mail'), findsNothing);
      expect(Core.account, p);
      expect(states(), {w: 'unlocked', p: 'locked'});

      await unlock(tester, 'Gate code');
      expect(find.text('Example Mail'), findsNothing);
      expect(find.text('Backup service'), findsNothing);
      expect(states(), {w: 'unlocked', p: 'unlocked'});

      // Back to work: still open, no master password.
      await openMenu(tester);
      await tester.tap(find.text(work).last);
      await opened(tester, 'Example Mail');
      expect(find.text('Gate code'), findsNothing);
      expect(Core.account, w);

      // Lock locks the one shown; the other stays open.
      await tester.tap(find.text('Lock'));
      await tester.pumpAndSettle();
      expect(find.text('Unlock PassBundle'), findsOneWidget);
      expect(states(), {w: 'locked', p: 'unlocked'});

      // From the lock screen, the open one is a switch away.
      await tester.tap(find.text('Switch account'));
      await tester.pumpAndSettle();
      await tester.tap(find.text(home).last);
      await opened(tester, 'Gate code');
      expect(Core.account, p);
    },
  );

  testWidgets(
    'adding an account goes to the sign-in screen, and back to the one shown as it was',
    (tester) async {
      wideWindow(tester);
      final [w, p] = openAccounts();
      await tester.pumpWidget(const PassBundleApp());
      await unlock(tester, 'Example Mail');
      await openMenu(tester);
      await tester.tap(find.text('Add account'));
      await tester.pumpAndSettle();
      expect(find.text('Add an account'), findsOneWidget);
      expect(find.widgetWithText(TextField, 'Email'), findsOneWidget);
      // Nothing of the one shown is on the sign-in screen.
      expect(find.text('Example Mail'), findsNothing);
      expect(Core.account, '');

      await tester.tap(find.text('Cancel'));
      await opened(tester, 'Example Mail');
      expect(Core.account, w);
      expect(states(), {w: 'unlocked', p: 'locked'});
    },
  );

  testWidgets('signing out of one account leaves the other', (tester) async {
    wideWindow(tester);
    final [w, p] = openAccounts();
    await tester.pumpWidget(const PassBundleApp());
    await unlock(tester, 'Example Mail');
    await openMenu(tester);
    await tester.tap(find.text('Sign out of this account'));
    // It asks the agent whether biometric unlock is on first.
    await pumpUntil(tester, find.text('Sign out?'));
    await tester.pumpAndSettle();
    // It says which account's vault leaves.
    expect(find.textContaining('the vault of $work'), findsOneWidget);
    await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
    await pumpUntil(tester, find.text('Unlock PassBundle'));
    await tester.pumpAndSettle();
    // The personal account is the one shown now, and the only one.
    expect(find.text(home), findsOneWidget);
    expect(find.text('Switch account'), findsNothing);
    expect(states(), {p: 'locked'});
    expect(Core.account, p);
    expect(Directory('${Core.folder}/accounts/$w').existsSync(), isFalse);
    await unlock(tester, 'Gate code');
  });

  testWidgets('with five accounts, adding one waits for signing out of one', (
    tester,
  ) async {
    wideWindow(tester);
    openAccounts(more: 3);
    await tester.pumpWidget(const PassBundleApp());
    await unlock(tester, 'Example Mail');
    await openMenu(tester);
    expect(find.text('more3@example.test'), findsOneWidget);
    expect(find.text('Add account'), findsNothing);
    expect(find.text('Up to 5 accounts on this device'), findsOneWidget);
    // It can't be chosen.
    await tester.tap(find.text('Up to 5 accounts on this device'));
    await tester.pumpAndSettle();
    expect(find.text('Add an account'), findsNothing);
  });

  testWidgets(
    'on a phone, Settings lists the accounts, and the vault\'s header switches between them',
    (tester) async {
      phoneWindow(tester);
      PhoneStandIn(tester);
      final [w, p] = openAccounts();
      // Not about biometric unlock (unlock_offer_test.dart is).
      declineOffer();
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
      await tester.pumpAndSettle();
      // Down Settings to its accounts, then each held in view to be read
      // and tapped (a small screen shows a few rows at a time).
      await tester.scrollUntilVisible(
        find.text(home),
        200,
        scrollable: find
            .descendant(
              of: find.byType(SettingsPage),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      await tester.pumpAndSettle();
      expect(find.text(work), findsOneWidget);
      expect(find.text('Add account'), findsOneWidget);
      expect(
        find.textContaining('These settings are for $work'),
        findsOneWidget,
      );
      await tester.ensureVisible(find.text(home));
      await tester.pumpAndSettle();
      await tester.tap(find.text(home));
      await tester.pumpAndSettle();
      expect(find.text('Unlock PassBundle'), findsOneWidget);
      expect(find.text(home), findsOneWidget);
      expect(states(), {w: 'unlocked', p: 'locked'});

      // Back to work, from the lock screen: open as it was.
      await tester.tap(find.text('Switch account'));
      await tester.pumpAndSettle();
      await tester.tap(find.text(work).last);
      await opened(tester, 'Logins');
      // The header's account button has them too.
      await tester.tap(find.byTooltip('Accounts'));
      await tester.pumpAndSettle();
      expect(find.text(home), findsOneWidget);
      expect(find.text('Add account'), findsOneWidget);
    },
  );

  testWidgets(
    'an install from before several accounts is carried over as its one account',
    (tester) async {
      wideWindow(tester);
      // As an earlier build kept it: in the app's own folder.
      final dir = openSample();
      expect(File('$dir/settings.json').existsSync(), isFalse);
      expect(File('$dir/sync.json').existsSync(), isFalse);
      final folders = Directory('$dir/accounts').listSync();
      expect(folders, hasLength(1));
      expect(File('${folders.single.path}/sync.json').existsSync(), isTrue);
      expect(Core().accounts()['accounts'], hasLength(1));
      await tester.pumpWidget(const PassBundleApp());
      expect(find.text(work), findsOneWidget);
      // One account: nothing to switch to.
      expect(find.text('Switch account'), findsNothing);
      await unlock(tester, 'Gate code');
    },
  );
}
