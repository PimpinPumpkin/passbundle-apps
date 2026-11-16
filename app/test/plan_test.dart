// Settings' Plan group (plan.dart; DESIGN.md in the meta repo, "Plans and
// billing"): the person's own plan and each organization's they own or are
// an admin of, with the state and dates in the web vault's words, in each
// state; nothing at all where the server doesn't enforce plans; and never a
// price, a way to buy, or a link to pay. Then the same through the real core
// (PB_CORE_LIB), from the sample vault's kept sync with `keykeeperPlan` in
// it, as PassBundle's server sends it.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/plan.dart';
import 'package:passbundle/theme.dart';

import 'support.dart';

/// Noon UTC, so each is the same day wherever the test runs.
const nov15 = '2026-11-15T12:00:00.000000Z';
const nov29 = '2026-11-29T12:00:00.000000Z';
const dec15 = '2026-12-15T12:00:00.000000Z';
const jun1 = '2027-06-01T12:00:00.000000Z';

/// `keykeeperPlan` as the server sends it to whoever manages the plan, with
/// what a test changes.
Map<String, dynamic> plan([Map<String, dynamic> fields = const {}]) => {
  'plan': 'business',
  'state': 'active',
  'freeUntil': null,
  'freeWhy': null,
  'until': null,
  'renews': null,
  'endsAt': null,
  'storageGb': 12,
  'people': null,
  'coveredBy': null,
  'canManage': true,
  ...fields,
};

/// What the core answers (`Core.plans`) where plans are enforced.
Map<String, dynamic> plans({
  Map<String, dynamic>? own,
  List<Map<String, dynamic>> organizations = const [],
  bool premiumFromOrganization = false,
}) => {
  'enforced': true,
  'own': own ?? plan({'plan': 'free', 'state': null, 'storageGb': 0}),
  'premiumFromOrganization': premiumFromOrganization,
  'organizations': organizations,
};

Map<String, dynamic> organization(String name, Map<String, dynamic> plan) => {
  'id': name.toLowerCase().replaceAll(' ', '-'),
  'name': name,
  'role': 'owner',
  'plan': plan,
};

/// The lines under one row of the Plan group.
List<String> linesOf(Map<String, dynamic> answer, String key) =>
    planRows(answer).firstWhere((r) => r.key == key).lines;

Future<void> showPlans(WidgetTester tester, Map<String, dynamic> answer) =>
    tester.pumpWidget(
      MaterialApp(
        theme: passBundleTheme(Brightness.light),
        home: Scaffold(
          body: SingleChildScrollView(child: PlanSettings(plans: answer)),
        ),
      ),
    );

void main() {
  setUpAll(loadFonts);

  group('the words', () {
    test('a date in words', () {
      expect(planDate(dec15), 'December 15, 2026');
      expect(planDate('not a date'), 'not a date');
    });

    test('Free', () {
      final row = planRows(plans()).single;
      expect(row.key, 'plan-own');
      expect(row.title, 'Free');
      expect(row.lines, isEmpty);
    });

    test('Free, with Premium through an organization', () {
      expect(linesOf(plans(premiumFromOrganization: true), 'plan-own'), [
        "You get Premium's features through an organization or an MSP you belong to.",
      ]);
    });

    test('Premium that renews, and one that was canceled', () {
      expect(
        linesOf(
          plans(
            own: plan({'plan': 'premium', 'until': dec15, 'renews': true}),
          ),
          'plan-own',
        ),
        ['Paid through December 15, 2026', 'Renews on December 15, 2026'],
      );
      expect(
        linesOf(
          plans(
            own: plan({'plan': 'premium', 'until': dec15, 'renews': false}),
          ),
          'plan-own',
        ),
        ['Paid through December 15, 2026', 'Ends on December 15, 2026'],
      );
    });

    test('a payment due, with and without the end the server says', () {
      final due = plan({
        'plan': 'premium',
        'state': 'paymentDue',
        'until': nov15,
        'renews': true,
      });
      expect(linesOf(plans(own: due), 'plan-own'), [
        'Payment due. Paid through November 15, 2026.',
      ]);
      expect(linesOf(plans(own: {...due, 'endsAt': nov29}), 'plan-own'), [
        'Payment due. Paid through November 15, 2026; it ends on November 29, 2026.',
      ]);
    });

    test('Premium that has ended', () {
      final ended = plan({'plan': 'premium', 'state': 'ended', 'until': nov15});
      expect(linesOf(plans(own: {...ended, 'endsAt': nov29}), 'plan-own'), [
        'Ended on November 29, 2026',
        "You're on Free now. Everything you saved stays, and files you attached can still be downloaded and removed.",
      ]);
      expect(linesOf(plans(own: ended), 'plan-own').first, 'Ended');
    });

    test("an organization's trial, and a free offer", () {
      final answer = plans(
        organizations: [
          organization(
            'Acme Dental',
            plan({
              'state': 'trial',
              'freeUntil': dec15,
              'freeWhy': 'trial',
              'renews': false,
            }),
          ),
          organization(
            'Baker Law',
            plan({'state': 'trial', 'freeUntil': jun1, 'freeWhy': 'offer'}),
          ),
        ],
      );
      final rows = planRows(answer);
      expect(rows.map((r) => r.title), ['Free', 'Acme Dental', 'Baker Law']);
      expect(rows[1].lines, ['Business', 'Trial until December 15, 2026']);
      expect(rows[2].lines, ['Business', 'Free until June 1, 2027']);
    });

    test('a client of an MSP, whose owners see no dates', () {
      final answer = plans(
        organizations: [
          organization(
            'Acme Dental',
            plan({
              'plan': 'msp',
              'state': 'trial',
              'canManage': false,
              'coveredBy': {'id': 'msp-1', 'name': 'Contoso IT'},
            }),
          ),
        ],
      );
      expect(linesOf(answer, 'plan-org-acme-dental'), [
        'MSP seat pool',
        'Trial',
        'Covered by Contoso IT',
      ]);
    });

    test("an organization's plan that has ended", () {
      final answer = plans(
        organizations: [
          organization(
            'Acme Dental',
            plan({'plan': 'families', 'state': 'ended', 'endsAt': nov29}),
          ),
        ],
      );
      expect(linesOf(answer, 'plan-org-acme-dental'), [
        'Families',
        'Ended on November 29, 2026',
        "This organization's plan has ended, so nothing new can be added. Everything in it can still be opened, changed, exported, and deleted.",
      ]);
    });

    test('an organization with no plan, and one active for an admin', () {
      final answer = plans(
        organizations: [
          organization('Acme Dental', plan({'plan': null, 'state': null})),
          organization('Baker Law', plan({'canManage': false})),
        ],
      );
      expect(linesOf(answer, 'plan-org-acme-dental'), ['No plan']);
      expect(linesOf(answer, 'plan-org-baker-law'), ['Business', 'Active']);
    });

    test('nothing where the server doesn\'t enforce plans', () {
      expect(planRows({'enforced': false}), isEmpty);
      expect(planRows(const {}), isEmpty);
      // What the core answers with no account.
      expect(planRows({'ok': false, 'error': 'Signed out'}), isEmpty);
    });
  });

  group('the Plan group in Settings', () {
    testWidgets('shows each plan, and never a price or a way to pay', (
      tester,
    ) async {
      await showPlans(
        tester,
        plans(
          own: plan({
            'plan': 'premium',
            'state': 'paymentDue',
            'until': nov15,
            'renews': true,
            'endsAt': nov29,
          }),
          organizations: [
            organization(
              'Acme Dental',
              plan({'state': 'trial', 'freeUntil': dec15, 'freeWhy': 'trial'}),
            ),
          ],
        ),
      );
      expect(find.text('Plan'), findsOneWidget);
      expect(find.text('Premium'), findsOneWidget);
      expect(
        find.text(
          'Payment due. Paid through November 15, 2026; it ends on November 29, 2026.',
        ),
        findsOneWidget,
      );
      expect(find.text('Acme Dental'), findsOneWidget);
      expect(
        find.text('Business\nTrial until December 15, 2026'),
        findsOneWidget,
      );
      // No buying, no prices, and nowhere to pay (the design's "Not yet").
      expect(find.byType(TextButton), findsNothing);
      expect(find.byType(FilledButton), findsNothing);
      expect(find.byType(OutlinedButton), findsNothing);
      expect(find.byIcon(Icons.open_in_new_rounded), findsNothing);
      for (final word in [
        '\$',
        'Buy',
        'Upgrade',
        'price',
        'Set up payment',
        'Manage payment',
      ]) {
        expect(find.textContaining(word), findsNothing, reason: word);
      }
    });

    testWidgets('shows a Free account as Free', (tester) async {
      await showPlans(tester, plans());
      expect(find.byKey(const ValueKey('plan')), findsOneWidget);
      expect(find.text('Free'), findsOneWidget);
    });

    testWidgets('shows an organization whose plan has ended', (tester) async {
      await showPlans(
        tester,
        plans(
          organizations: [
            organization(
              'Acme Dental',
              plan({'state': 'ended', 'endsAt': nov29}),
            ),
          ],
        ),
      );
      expect(
        find.textContaining('Ended on November 29, 2026'),
        findsOneWidget,
      );
      expect(
        find.textContaining(
          "This organization's plan has ended, so nothing new can be added.",
        ),
        findsOneWidget,
      );
    });

    testWidgets('is left out where the server doesn\'t enforce plans', (
      tester,
    ) async {
      await showPlans(tester, {'enforced': false});
      expect(find.byKey(const ValueKey('plan')), findsNothing);
      expect(find.text('Plan'), findsNothing);
    });
  });

  group('through the real core', () {
    /// The sample vault, locked, with `keykeeperPlan` on its profile and on
    /// its organization (which the sample's person owns), as given.
    void openSampleWithPlans(
      Map<String, dynamic>? own,
      Map<String, dynamic>? contoso,
    ) {
      final dir = Directory.systemTemp.createTempSync('pb-app-test-').path;
      File('test/sample/settings.json').copySync('$dir/settings.json');
      final sync =
          jsonDecode(File('test/sample/sync.json').readAsStringSync())
              as Map<String, dynamic>;
      final profile = sync['profile'] as Map<String, dynamic>;
      if (own != null) profile['keykeeperPlan'] = own;
      for (final org
          in (profile['organizations'] as List).cast<Map<String, dynamic>>()) {
        if (contoso != null) org['keykeeperPlan'] = contoso;
      }
      File('$dir/sync.json').writeAsStringSync(jsonEncode(sync));
      Core().openAt(dir);
      machineDoesntLock(false);
    }

    Future<void> unlockAndOpenSettings(WidgetTester tester) async {
      tester.view.physicalSize = const Size(1280, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Backup service'));
      // The sync on opening is done (there's no server), so the core is free.
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      await tester.tap(find.byTooltip('Settings'));
      await tester.pump();
      await pumpUntil(tester, find.text('Lock after'));
      await pumpUntilGone(
        tester,
        find.byKey(const ValueKey('unlock-loading')),
      );
      await tester.pump(const Duration(milliseconds: 400));
    }

    testWidgets("Settings shows the plans from the vault's last sync", (
      tester,
    ) async {
      openSampleWithPlans(
        {'enforced': true, ...plan({'plan': 'free', 'state': null})},
        {
          'enforced': true,
          ...plan({
            'state': 'trial',
            'freeUntil': dec15,
            'freeWhy': 'trial',
            'renews': false,
          }),
        },
      );
      await unlockAndOpenSettings(tester);
      final group = find.byKey(const ValueKey('plan'), skipOffstage: false);
      expect(group, findsOneWidget);
      Finder inGroup(String text) => find.descendant(
        of: group,
        matching: find.text(text, skipOffstage: false),
        skipOffstage: false,
      );
      expect(inGroup('Free'), findsOneWidget);
      expect(inGroup('Contoso'), findsOneWidget);
      expect(inGroup('Business\nTrial until December 15, 2026'), findsOneWidget);
    });

    testWidgets('Settings has no Plan group where the server doesn\'t enforce plans', (
      tester,
    ) async {
      openSampleWithPlans({'enforced': false}, {'enforced': false});
      await unlockAndOpenSettings(tester);
      expect(
        find.byKey(const ValueKey('plan'), skipOffstage: false),
        findsNothing,
      );
    });
  });
}
