// Expiration reminders on a phone while PassBundle is closed (docs/phones.md
// in the meta repo, "Expiration reminders while the app is closed"). The
// vault reminds only while it's open and unlocked (core/src/expiration.rs,
// vault.dart), and a phone app is mostly closed, so the phone says them
// itself. After unlocking, after each sync, and when items change, the core
// works out the coming days' reminders for every account on the phone (each
// unlocked one's again, a locked one's as they were last), added up by day,
// and the phone's own code schedules a notification for each of those days
// at 9:00 (Reminders.kt on Android, Reminders.swift on iOS). It says how
// many items, never which, in words the phone's code makes from two numbers.
// Each plan replaces the last; signing out of an account takes its days off,
// and turning the setting off takes them all. A day the phone said counts as
// shown, so the vault doesn't say it again; a day it didn't (the phone was
// off) the vault says as before. Tapping one opens Expiring soon, after
// unlocking. Nothing leaves the phone.
//
// The setting, in Settings, is the device's, for every account on it: on by
// default once the phone allows PassBundle's notifications, off if it
// doesn't, with where to allow them. The phone's permission is asked for
// once: the first time there's a reminder to schedule (a sheet says what it
// is first), or when the person turns the setting on.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

/// What the phone says about PassBundle's notifications.
enum NotificationAccess {
  /// The phone shows them.
  allowed,

  /// The person said no, or turned them off in the phone's settings.
  refused,

  /// Not asked yet (Android 13 and later, and iOS, ask once).
  notAsked,
}

/// What the switch says, in the setting and the sheet.
const closedRemindersTitle =
    'Remind me when items expire, even when the app is closed';

/// What a reminder is, in a line.
const closedRemindersLine =
    'At 9:00 on each reminder day. It says how many items, never which.';

/// Where to allow PassBundle's notifications in the phone's own Settings.
String get allowNotificationsWhere => isIPhone
    ? 'Notifications are off for PassBundle. To get reminders, turn them on in Settings, Notifications, PassBundle.'
    : 'Notifications are off for PassBundle. To get reminders, turn them on in Settings, Apps, PassBundle, Notifications.';

/// A call to the phone's own code (MainActivity.kt, AppDelegate.swift), or
/// null where it doesn't answer: a computer, or a test's stand-in without it.
Future<T?> _onPhone<T>(String method, [Object? arguments]) async {
  if (!isPhone) return null;
  try {
    return await Core.platform.invokeMethod<T>(method, arguments);
  } on PlatformException {
    return null;
  } on MissingPluginException {
    return null;
  }
}

NotificationAccess? _access(String? answer) => switch (answer) {
  'allowed' => NotificationAccess.allowed,
  'refused' => NotificationAccess.refused,
  'notAsked' => NotificationAccess.notAsked,
  _ => null,
};

/// Whether the phone shows PassBundle's notifications; null off a phone.
Future<NotificationAccess?> notificationAccess() async =>
    _access(await _onPhone<String>('notificationAccess'));

/// The phone's own question about PassBundle's notifications, once, and its
/// answer. On Android 12 and earlier, which doesn't ask, what it is.
Future<NotificationAccess?> askForNotifications() async =>
    _access(await _onPhone<String>('askNotifications'));

/// The phone's own settings for PassBundle's notifications: true if they
/// opened.
Future<bool> openNotificationSettings() async =>
    await _onPhone<bool>('openNotificationSettings') ?? false;

/// The days ("YYYY-MM-DD") whose reminder the phone said, recently.
Future<List<String>> remindersDelivered() async => [
  for (final day in await _onPhone<List>('remindersDelivered') ?? const [])
    if (day is String) day,
];

/// Whether PassBundle was opened from a reminder since this was last asked.
Future<bool> openedFromReminder() async =>
    await _onPhone<bool>('remindersOpened') ?? false;

/// The person's own choices about the setting: [off] when they turned it
/// off (or said "Not now"), and [asked] once the sheet has asked.
@immutable
class ClosedRemindersChoice {
  const ClosedRemindersChoice({this.off = false, this.asked = false});

  final bool off;
  final bool asked;
}

/// Where they're kept: beside the vault, in the app's own folder, for every
/// account on the phone.
File? get _kept {
  final dir = Core.folder;
  return dir == null ? null : File('$dir/closed-reminders.json');
}

ClosedRemindersChoice readClosedRemindersChoice() {
  try {
    final json = jsonDecode(_kept!.readAsStringSync()) as Map<String, dynamic>;
    return ClosedRemindersChoice(
      off: json['off'] == true,
      asked: json['asked'] == true,
    );
  } on Object {
    return const ClosedRemindersChoice();
  }
}

void _keep(ClosedRemindersChoice choice) {
  try {
    _kept?.writeAsStringSync(
      jsonEncode({'off': choice.off, 'asked': choice.asked}),
    );
  } on FileSystemException {
    // Then the default holds: on, once the phone allows it.
  }
}

/// Whether the phone says reminders while PassBundle is closed.
bool closedRemindersOn(NotificationAccess? access) =>
    access == NotificationAccess.allowed && !readClosedRemindersChoice().off;

/// Works out the reminders again and hands them to the phone, in place of
/// what it had: after unlocking, each sync and change, signing out, and the
/// setting changing. One at a time; a call while one runs runs once more
/// after it, with what's newest then. Nothing off a phone.
Future<void> scheduleClosedReminders() async {
  if (!isPhone) return;
  if (_scheduling case final running?) {
    _again = true;
    return running;
  }
  final done = Completer<void>();
  _scheduling = done.future;
  try {
    do {
      _again = false;
      await _scheduleOnce();
    } while (_again);
  } on Object {
    // The next unlock, sync, or change tries again.
  } finally {
    _scheduling = null;
    done.complete();
  }
}

Future<void>? _scheduling;
bool _again = false;

Future<void> _scheduleOnce() async {
  final access = await notificationAccess();
  // The phone's code doesn't answer: nothing to hand it.
  if (access == null) return;
  final plan = await Core.reminderPlan();
  if (!plan.ok) return;
  final days = closedRemindersOn(access)
      ? (plan.body['days'] as List? ?? const [])
      : const [];
  await _onPhone<Object>('scheduleReminders', {'days': days});
}

/// The setting turned on or off by the person: off takes every day the
/// phone had waiting, on asks the phone first if it hasn't been asked.
/// What the phone says after.
Future<NotificationAccess?> turnClosedReminders(bool on) async {
  final was = readClosedRemindersChoice();
  var access = await notificationAccess();
  if (on && access == NotificationAccess.notAsked) {
    access = await askForNotifications();
  }
  _keep(ClosedRemindersChoice(off: !on, asked: was.asked || on));
  await scheduleClosedReminders();
  return access;
}

/// As the vault opens on a phone: the first time there's a reminder to
/// schedule, and the phone hasn't been asked about notifications, a sheet
/// says what they are, and "Turn on" asks the phone. Once: "Not now" turns
/// the setting off, and Settings has it.
Future<void> offerClosedReminders(BuildContext context) async {
  if (!isPhone) return;
  final choice = readClosedRemindersChoice();
  if (choice.asked || choice.off) return;
  if (await notificationAccess() != NotificationAccess.notAsked) return;
  final plan = await Core.reminderPlan();
  final days = plan.body['days'] as List? ?? const [];
  if (!plan.ok || days.isEmpty || !context.mounted) return;
  final access = await showModalBottomSheet<NotificationAccess>(
    context: context,
    isScrollControlled: true,
    builder: (_) => const ClosedRemindersOffer(),
  );
  // Asked once, whatever the answer; "Not now" (or closing the sheet) is off.
  _keep(ClosedRemindersChoice(off: access == null, asked: true));
  await scheduleClosedReminders();
}

/// The sheet: what the reminders are, "Turn on" (the phone asks), and "Not
/// now". It answers with what the phone said, or null for "Not now".
class ClosedRemindersOffer extends StatefulWidget {
  const ClosedRemindersOffer({super.key});

  @override
  State<ClosedRemindersOffer> createState() => _ClosedRemindersOfferState();
}

class _ClosedRemindersOfferState extends State<ClosedRemindersOffer> {
  bool working = false;

  Future<void> turnOn() async {
    setState(() => working = true);
    final access = await askForNotifications();
    if (!mounted) return;
    Navigator.of(context).pop(access ?? NotificationAccess.refused);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final scheme = Theme.of(context).colorScheme;
    return PopScope(
      // Not while the phone is asking.
      canPop: !working,
      child: SafeArea(
        child: SingleChildScrollView(
          padding: const EdgeInsets.fromLTRB(24, 0, 24, 16),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Center(
                child: Container(
                  width: 64,
                  height: 64,
                  decoration: BoxDecoration(
                    color: c.tint,
                    shape: BoxShape.circle,
                  ),
                  child: Icon(
                    Icons.notifications_outlined,
                    size: 34,
                    color: scheme.primary,
                  ),
                ),
              ),
              const SizedBox(height: 16),
              const Text(
                'Get reminders when items expire?',
                textAlign: TextAlign.center,
                style: TextStyle(fontSize: 22, fontWeight: FontWeight.w700),
              ),
              const SizedBox(height: 8),
              Text(
                'PassBundle can remind you at 9:00 on each reminder day, even '
                'when the app is closed. A reminder says how many items, '
                'never which. You can change this in Settings.',
                textAlign: TextAlign.center,
                style: TextStyle(color: c.muted, fontSize: 15, height: 1.35),
              ),
              const SizedBox(height: 24),
              SizedBox(
                height: 52,
                child: WorkingButton(
                  key: const ValueKey('closed-reminders-turn-on'),
                  label: 'Turn on',
                  working: working,
                  onPressed: turnOn,
                ),
              ),
              const SizedBox(height: 8),
              SizedBox(
                height: 48,
                child: TextButton(
                  onPressed: working
                      ? null
                      : () => Navigator.of(context).pop(),
                  child: const Text('Not now'),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The setting, in Settings on a phone: a switch while the phone allows
/// PassBundle's notifications or hasn't been asked; where to allow them once
/// it's been told no, with a way there. Asked again when the app is back in
/// front, from the phone's settings or anywhere else.
class ClosedRemindersSetting extends StatefulWidget {
  const ClosedRemindersSetting({super.key});

  @override
  State<ClosedRemindersSetting> createState() => _ClosedRemindersSettingState();
}

class _ClosedRemindersSettingState extends State<ClosedRemindersSetting> {
  NotificationAccess? access;
  bool loaded = false;
  bool working = false;

  late final AppLifecycleListener lifecycle = AppLifecycleListener(
    onResume: refresh,
  );

  @override
  void initState() {
    super.initState();
    lifecycle;
    refresh();
  }

  @override
  void dispose() {
    lifecycle.dispose();
    super.dispose();
  }

  Future<void> refresh() async {
    final next = await notificationAccess();
    if (!mounted) return;
    final changed = loaded && next != access;
    setState(() {
      access = next;
      loaded = true;
    });
    // Allowed (or refused) in the phone's settings meanwhile: the phone's
    // reminders follow.
    if (changed) unawaited(scheduleClosedReminders());
  }

  Future<void> turn(bool on) async {
    setState(() => working = true);
    final next = await turnClosedReminders(on);
    if (!mounted) return;
    setState(() {
      working = false;
      access = next;
    });
  }

  @override
  Widget build(BuildContext context) {
    final access = this.access;
    // Not a phone that answers, or not yet.
    if (access == null) return const SizedBox.shrink();
    final rows = <Widget>[];
    if (access == NotificationAccess.refused) {
      rows.add(
        ListTile(
          key: const ValueKey('closed-reminders'),
          title: const Text(closedRemindersTitle),
          subtitle: Text(allowNotificationsWhere),
          isThreeLine: true,
        ),
      );
      rows.add(
        ListTile(
          key: const ValueKey('closed-reminders-settings'),
          title: const Text('Open notification settings'),
          trailing: const Icon(Icons.open_in_new_rounded, size: 20),
          onTap: openNotificationSettings,
        ),
      );
    } else {
      rows.add(
        SwitchListTile(
          key: const ValueKey('closed-reminders'),
          value: closedRemindersOn(access),
          onChanged: working ? null : turn,
          title: const Text(closedRemindersTitle),
          subtitle: const Text(closedRemindersLine),
        ),
      );
    }
    return SettingsGroup(title: 'Reminders', children: rows);
  }
}
