// Two-step codes need Premium (core/src/codes.rs; DESIGN.md in the meta
// repo, "Plans and billing", decision 2), against the real core: an item's
// view shows a login's current code, with its Copy, only where the person
// has Premium, of their own or through an organization, or the login's
// organization gives codes (the core's tests cover that one). Otherwise it
// says so where the code would be, and the editor still has the key. Where
// plans aren't enforced, the server says everyone has Premium, and codes
// show as they always have.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/main.dart';

import 'support.dart';

const needsPremium = 'Two-step login codes are part of Premium.';

/// The sample's login with a key: "Backup service", and its key.
const login = 'Backup service';
const key = 'GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ';

/// The sample vault unlocked on a computer, its profile saying `profile`.
Future<void> unlock(
  WidgetTester tester, {
  Map<String, dynamic> profile = const {},
}) async {
  tester.view.physicalSize = const Size(1280, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  openSample(profile: profile);
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text(login));
  // The background sync finishes (offline: no server behind the sample).
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
  await tester.tap(find.text(login).last);
  await tester.pumpAndSettle();
}

/// Whether the open item shows its code: the code and its Copy, or the
/// sentence in their place.
void showsCode(bool shown) {
  expect(find.text('One-time code'), findsOneWidget);
  expect(
    find.byTooltip('Copy one-time code'),
    shown ? findsOneWidget : findsNothing,
  );
  expect(find.text(needsPremium), shown ? findsNothing : findsOneWidget);
  // Its other secrets are as ever.
  expect(find.byTooltip('Copy password'), findsOneWidget);
}

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'without Premium, a login says its code is part of Premium, offers no Copy, and its editor still has the key',
    (tester) async {
      await unlock(
        tester,
        profile: {'premium': false, 'premiumFromOrganization': false},
      );
      showsCode(false);

      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
      await tester.pumpAndSettle();
      final field = tester.widget<TextField>(
        find.widgetWithText(TextField, 'One-time code secret (optional)'),
      );
      expect(field.controller!.text, key);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
    },
  );

  testWidgets('with Premium through an organization, the code shows', (
    tester,
  ) async {
    await unlock(
      tester,
      profile: {'premium': false, 'premiumFromOrganization': true},
    );
    showsCode(true);
  });

  testWidgets('with Premium of their own, the code shows', (tester) async {
    await unlock(
      tester,
      profile: {
        'premium': true,
        'premiumFromOrganization': false,
        'keykeeperPlan': {
          'enforced': true,
          'plan': 'premium',
          'state': 'active',
          'canManage': true,
        },
      },
    );
    showsCode(true);
  });

  testWidgets(
    'where plans aren\'t enforced, the code shows as it always has',
    (tester) async {
      // What such a server says of everyone.
      await unlock(
        tester,
        profile: {
          'premium': true,
          'premiumFromOrganization': false,
          'keykeeperPlan': {'enforced': false},
        },
      );
      showsCode(true);
    },
  );
}
