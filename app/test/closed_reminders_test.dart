// Expiration reminders on a phone while PassBundle is closed
// (closed_reminders.dart), against the real core (PB_CORE_LIB) with a
// stand-in for the phone's own code (support.dart, NotificationsStandIn): the
// first time there's a reminder to schedule, a sheet asks, and "Turn on" asks
// the phone; the phone is handed each coming day's counts, never a name;
// "Not now" and the phone saying no leave it off, and Settings says where to
// allow notifications; Settings turns it on and off; signing out takes the
// days away; a day the phone said isn't said again in the vault, and one it
// didn't is; and a tapped reminder opens Expiring soon.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/closed_reminders.dart';
import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/widgets.dart';

import 'support.dart';

/// The sample's own login (the core's vectors).
const exampleMail = '3d4e5f60-7182-4930-abc1-d2e3f4051627';

const offered = 'Get reminders when items expire?';
const setting = ValueKey('closed-reminders');

/// Example Mail and Gate code both expire in 10 days; the gate code
/// reminds 7 and 1 day before. Today Example Mail is at its 30 days (the
/// vault says it); the phone gets the days after: both at 7 days, both at
/// 1, both once expired.
Map<String, Map<String, String>> get twoExpiring => {
  exampleMail: {'expires': isoInDays(10)},
  gateCode: {'expires': isoInDays(10), 'remind': '7,1'},
};

List<Map<String, dynamic>> get twoExpiringDays => [
  {'date': isoInDays(3), 'soon': 2, 'expired': 0},
  {'date': isoInDays(9), 'soon': 2, 'expired': 0},
  {'date': isoInDays(11), 'soon': 0, 'expired': 2},
];

/// A phone with the sample vault, these items given dates, and [phone]'s
/// notifications; unlocked with the master password. [before] runs on the
/// app's folder before the app opens.
Future<NotificationsStandIn> unlockOnPhone(
  WidgetTester tester,
  Map<String, Map<String, String>> expiring, {
  NotificationsStandIn? phone,
  void Function(String dir)? before,
}) async {
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final dir = openSample();
  Core().testExpiration(expiring);
  addTearDown(() => Core().testExpiration(const {}));
  final notifications = phone ?? NotificationsStandIn();
  // No biometrics here, so the vault doesn't offer them first.
  PhoneStandIn(tester, status: 'unavailable', notifications: notifications);
  before?.call(dir);
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text('Logins'));
  return notifications;
}

/// Pumps until [done], giving slow core calls real time.
Future<void> pumpUntilTrue(
  WidgetTester tester,
  bool Function() done,
  String what,
) async {
  final end = DateTime.now().add(const Duration(seconds: 20));
  while (DateTime.now().isBefore(end)) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
    await tester.pump();
    if (done()) return;
  }
  throw TestFailure('Never happened: $what');
}

/// The sheet that asks, up all the way: it's there from the first frame of
/// sliding in, below the foot of the screen, where a tap misses it.
Future<void> sheetUp(WidgetTester tester) async {
  await pumpUntil(tester, find.text(offered));
  await tester.pump(const Duration(seconds: 1));
}

/// A phone's tab, from the bar at its foot.
Future<void> tab(WidgetTester tester, String name) async {
  await tester.tap(
    find.descendant(of: find.byType(NavigationBar), matching: find.text(name)),
  );
  await tester.pumpAndSettle();
}

/// Settings, scrolled to the reminders' setting.
Future<void> reminderSetting(WidgetTester tester) async {
  await tab(tester, 'Settings');
  await pumpUntil(tester, find.byKey(setting));
  await tester.ensureVisible(find.byKey(setting));
  await tester.pumpAndSettle();
}

bool switchedOn(WidgetTester tester) =>
    tester.widget<SwitchListTile>(find.byKey(setting)).value;

/// Back in front, from the phone's settings or anywhere else.
Future<void> comeBack(WidgetTester tester) async {
  tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
  tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
  await tester.pump();
}

void main() {
  setUpAll(loadFonts);
  setUp(forgetClosedRemindersScheduling);

  testWidgets(
    'the first time there\'s a reminder to schedule, a sheet asks; Turn on asks the phone, which gets each day\'s counts and never a name',
    (tester) async {
      final phone = await unlockOnPhone(tester, twoExpiring);
      await sheetUp(tester);
      // Nothing for the phone before it's been asked.
      expect(phone.waiting ?? const [], isEmpty);
      expect(
        find.textContaining('A reminder says how many items, never which.'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const ValueKey('closed-reminders-turn-on')));
      await pumpUntilTrue(
        tester,
        () => phone.waiting?.isNotEmpty ?? false,
        'the phone handed its days',
      );
      await tester.pumpAndSettle();
      expect(phone.asked, 1);
      expect(find.text(offered), findsNothing);
      expect(phone.waiting, twoExpiringDays);
      // Dates and two numbers: nothing from an item, not even its id.
      final handed = jsonEncode(phone.scheduled);
      for (final what in ['Example Mail', 'Gate code', exampleMail, gateCode]) {
        expect(handed, isNot(contains(what)));
      }
      for (final day in phone.scheduled.expand((plan) => plan)) {
        expect(day.keys.toSet(), {'date', 'soon', 'expired'});
      }
      // Today's is the vault's own, at its top.
      await pumpUntil(tester, find.text('1 item expires soon'));

      // Settings has it on, for every account on the phone.
      await reminderSetting(tester);
      expect(switchedOn(tester), isTrue);
      expect(find.text(closedRemindersTitle), findsOneWidget);
      expect(find.text(closedRemindersLine), findsOneWidget);

      // Locked and unlocked again: asked once only.
      await tab(tester, 'Items');
      await tester.tap(find.byTooltip('Lock'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Logins'));
      await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
      await tester.pump(const Duration(seconds: 1));
      expect(find.text(offered), findsNothing);
      expect(phone.asked, 1);
      expect(phone.waiting, twoExpiringDays);
    },
  );

  testWidgets(
    'Not now leaves it off with nothing for the phone; Settings turns it on, which asks the phone, and off, which takes every day away',
    (tester) async {
      final phone = await unlockOnPhone(tester, twoExpiring);
      await sheetUp(tester);
      await tester.tap(find.text('Not now'));
      await tester.pumpAndSettle();
      await pumpUntilTrue(
        tester,
        () => phone.scheduled.isNotEmpty,
        'the phone told',
      );
      expect(phone.asked, 0);
      expect(phone.waiting, isEmpty);

      await reminderSetting(tester);
      expect(switchedOn(tester), isFalse);
      await tester.tap(find.byKey(setting));
      await pumpUntilTrue(
        tester,
        () => phone.waiting?.isNotEmpty ?? false,
        'the phone handed its days',
      );
      await tester.pumpAndSettle();
      expect(phone.asked, 1);
      expect(phone.waiting, twoExpiringDays);
      expect(switchedOn(tester), isTrue);

      await tester.tap(find.byKey(setting));
      await pumpUntilTrue(
        tester,
        () => phone.waiting!.isEmpty,
        'every day taken away',
      );
      await tester.pumpAndSettle();
      expect(switchedOn(tester), isFalse);
      // Turned off isn't asked about again.
      expect(phone.asked, 1);
    },
  );

  testWidgets(
    'the phone saying no leaves it off, Settings says where to allow notifications and opens it, and allowed there it\'s on',
    (tester) async {
      final phone = await unlockOnPhone(
        tester,
        twoExpiring,
        phone: NotificationsStandIn(allow: false),
      );
      await sheetUp(tester);
      await tester.tap(find.byKey(const ValueKey('closed-reminders-turn-on')));
      await pumpUntilTrue(
        tester,
        () => phone.scheduled.isNotEmpty,
        'the phone told',
      );
      // The sheet slides away on the test's clock, which only pumping with
      // time moves.
      await tester.pumpAndSettle();
      expect(find.text(offered), findsNothing);
      expect(phone.asked, 1);
      expect(phone.access, 'refused');
      expect(phone.waiting, isEmpty);

      await tab(tester, 'Settings');
      final where = find.text(allowNotificationsWhere);
      await pumpUntil(tester, where);
      expect(
        allowNotificationsWhere,
        contains('Settings, Apps, PassBundle, Notifications'),
      );
      // No switch that can't be on: where to allow them, and a way there.
      expect(
        find.byWidgetPredicate((w) => w is SwitchListTile && w.key == setting),
        findsNothing,
      );
      final open = find.byKey(const ValueKey('closed-reminders-settings'));
      await tester.ensureVisible(open);
      await tester.pumpAndSettle();
      await tester.tap(open);
      await tester.pumpAndSettle();
      expect(phone.settingsOpened, 1);

      // Allowed in the phone's settings, and back: on, as it is by default.
      phone.access = 'allowed';
      await comeBack(tester);
      await pumpUntilTrue(
        tester,
        () => phone.waiting?.isNotEmpty ?? false,
        'the phone handed its days',
      );
      await tester.pumpAndSettle();
      expect(phone.waiting, twoExpiringDays);
      expect(switchedOn(tester), isTrue);
      expect(where, findsNothing);
    },
  );

  testWidgets(
    'allowed already, it\'s on with no question, and signing out takes the account\'s days away',
    (tester) async {
      final phone = await unlockOnPhone(
        tester,
        twoExpiring,
        phone: NotificationsStandIn(access: 'allowed'),
      );
      await pumpUntilTrue(
        tester,
        () => phone.waiting?.length == 3,
        'the phone handed its days',
      );
      expect(phone.waiting, twoExpiringDays);
      await tester.pumpAndSettle();
      expect(find.text(offered), findsNothing);
      expect(phone.asked, 0);

      await tab(tester, 'Settings');
      final signOut = find.byKey(const ValueKey('sign-out-account'));
      await tester.ensureVisible(signOut);
      await tester.pumpAndSettle();
      await tester.tap(signOut);
      await pumpUntil(tester, find.text('Sign out?'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
      await pumpUntil(tester, find.text('Sign in to PassBundle'));
      await pumpUntilTrue(
        tester,
        () => phone.waiting!.isEmpty,
        'the account\'s days taken away',
      );
    },
  );

  group('a day the phone said', () {
    /// Today's reminder for Example Mail, as the phone had it waiting.
    void waitingToday(String dir) {
      File(
        '$dir/accounts/${Core.account}/schedule.json',
      ).writeAsStringSync(
        jsonEncode({
          isoInDays(0): {
            exampleMail: {'date': isoInDays(10), 'level': '30'},
          },
        }),
      );
    }

    final expiring = {
      exampleMail: {'expires': isoInDays(10)},
    };

    testWidgets('isn\'t said again in the vault', (tester) async {
      late String schedule;
      final phone = await unlockOnPhone(
        tester,
        expiring,
        phone: NotificationsStandIn(
          access: 'allowed',
          delivered: [isoInDays(0)],
        ),
        before: (dir) {
          waitingToday(dir);
          schedule = '$dir/accounts/${Core.account}/schedule.json';
        },
      );
      // The vault's check is done once today leaves what the phone has.
      await pumpUntilTrue(
        tester,
        () => !File(schedule).readAsStringSync().contains(isoInDays(0)),
        'today gone from the schedule',
      );
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 300)),
      );
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('expiration-reminder')), findsNothing);
      // And the phone's days are the ones after today.
      await pumpUntilTrue(
        tester,
        () =>
            (phone.waiting?.isNotEmpty ?? false) &&
            phone.waiting!.every((d) => d['date'] != isoInDays(0)),
        'the days after today handed to the phone',
      );
      expect(phone.waiting!.first, {
        'date': isoInDays(3),
        'soon': 1,
        'expired': 0,
      });
    });

    testWidgets('and one it didn\'t (it was off) the vault says', (
      tester,
    ) async {
      await unlockOnPhone(
        tester,
        expiring,
        phone: NotificationsStandIn(access: 'allowed'),
        before: waitingToday,
      );
      await pumpUntil(tester, find.text('1 item expires soon'));
    });
  });

  testWidgets(
    'a tapped reminder opens Expiring soon after unlocking, and while open, from any tab',
    (tester) async {
      final back = find.byTooltip('Back to all items');
      final phone = NotificationsStandIn(access: 'allowed')..opened = true;
      await unlockOnPhone(tester, {
        exampleMail: {'expires': isoInDays(10)},
      }, phone: phone);
      await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
      await pumpUntil(tester, back);
      await tester.pumpAndSettle();
      expect(
        find.descendant(of: find.byType(PageTitle), matching: find.text('Expiring soon')),
        findsOneWidget,
      );
      expect(find.text('Expires in 10 days'), findsOneWidget);
      expect(phone.opened, isFalse);

      // Back to every item, then Settings.
      await tester.tap(back);
      await tester.pumpAndSettle();
      expect(back, findsNothing);
      await reminderSetting(tester);

      // One tapped now: the items' tab, Expiring soon.
      phone.opened = true;
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        'passbundle/computer',
        const StandardMethodCodec().encodeMethodCall(
          const MethodCall('expiring'),
        ),
        (_) {},
      );
      await pumpUntil(tester, back);
      await tester.pumpAndSettle();
      expect(find.byKey(setting), findsNothing);
      expect(find.text('Expires in 10 days'), findsOneWidget);
      expect(phone.opened, isFalse);
    },
  );
}
