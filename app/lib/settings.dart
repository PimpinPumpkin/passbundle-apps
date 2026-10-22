// Settings: unlocking with Windows Hello or Touch ID, through the desktop
// agent and shared with the browser extension (docs/core.md, "Unlock with
// Windows Hello or Touch ID"), and when the app locks by itself ("Locking by
// itself"), within what an organization's policy allows ("Locking by
// policy").
import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

class SettingsDialog extends StatefulWidget {
  const SettingsDialog({super.key});

  @override
  State<SettingsDialog> createState() => _SettingsDialogState();
}

/// The waits offered, in minutes: -1 is "when the app closes", and on phones
/// 0 is "Immediately" (when the app leaves the screen).
final lockChoices = [if (isPhone) 0, 1, 5, 15, 30, 60, 240, -1];

String lockAfterName(int minutes) => switch (minutes) {
  -1 => 'When the app closes',
  0 => 'Immediately',
  1 => '1 minute',
  60 => '1 hour',
  final m when m % 60 == 0 => '${m ~/ 60} hours',
  final m => '$m minutes',
};

/// The organization's policy, said as the person reads it: what happens, and
/// when, or null if it doesn't limit the wait.
String? policyWait(Map<String, dynamic> policy) {
  final does = policy['signOut'] == true ? 'signs you out' : 'locks PassBundle';
  final max = policy['maxMinutes'] as int?;
  if (policy['immediately'] == true) {
    return isPhone
        ? 'Your organization $does when you leave PassBundle'
        : "Your organization $does when PassBundle's window closes or is minimized";
  }
  if (max != null) {
    return 'Your organization $does after ${lockAfterName(max).toLowerCase()} at most';
  }
  if (policy['signOut'] == true) {
    return 'Your organization signs you out instead of locking';
  }
  return null;
}

class _SettingsDialogState extends State<SettingsDialog> {
  final core = Core();
  late Map<String, dynamic> preferences = core.preferences();
  late final Map<String, dynamic> policy = core.lockPolicy();
  Map<String, dynamic>? status;
  bool working = false;
  String? note;

  void choose(Map<String, dynamic> changes) {
    final next = {...preferences, ...changes};
    final answer = core.setPreferences(next);
    setState(() {
      if (answer.ok) {
        preferences = next;
        note = null;
      } else {
        note = answer.error;
      }
    });
  }

  Widget lockAfter() {
    final chosen = (preferences['lockAfterMinutes'] as int?) ?? -1;
    // What it locks by, as the core holds the person's choice to the policy;
    // their own choice is kept as it is.
    final immediately = policy['immediately'] == true;
    final max = policy['maxMinutes'] as int?;
    final minutes = immediately
        ? 0
        : max == null
        ? chosen
        : chosen < 0
        ? max
        : chosen < max
        ? chosen
        : max;
    final allowed = lockChoices.where(
      (m) => immediately ? m == 0 : max == null || (m >= 0 && m <= max),
    );
    // A wait chosen or set some other way still shows as it is.
    final choices = {...allowed, minutes}.toList()
      ..sort((a, b) => (a < 0 ? 1 << 30 : a).compareTo(b < 0 ? 1 << 30 : b));
    final note = policyWait(policy);
    return ListTile(
      leading: const Icon(Icons.lock_clock_outlined),
      title: const Text('Lock after'),
      subtitle: Text(
        [
          isPhone ? 'Away from PassBundle' : 'Without using the computer',
          ?note,
        ].join('\n'),
      ),
      trailing: DropdownButton<int>(
        value: minutes,
        underline: const SizedBox.shrink(),
        onChanged: choices.length < 2
            ? null
            : (m) =>
                  choose({'lockAfterMinutes': m == null || m < 0 ? null : m}),
        items: [
          for (final m in choices)
            DropdownMenuItem(value: m, child: Text(lockAfterName(m))),
        ],
      ),
      contentPadding: EdgeInsets.zero,
    );
  }

  @override
  void initState() {
    super.initState();
    refresh();
  }

  Future<void> refresh() async {
    final next = await Core.biometricStatus();
    if (mounted) setState(() => status = next);
  }

  Future<void> turn(bool on, String method) async {
    if (!on) {
      final sure = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: Text('Turn off unlocking with $method?'),
          content: Text(
            isPhone
                ? 'You can turn it on again here any time.'
                : 'It turns off in your browser extension too.',
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(context, true),
              child: const Text('Turn off'),
            ),
          ],
        ),
      );
      if (sure != true) return;
    }
    setState(() {
      working = true;
      note = null;
    });
    final answer = await whileBusy(
      on ? Core.enrollBiometric : Core.removeBiometric,
    );
    if (!mounted) return;
    setState(() {
      working = false;
      if (!answer.ok) {
        note = answer.error;
      } else if (on && answer.body['on'] != true) {
        note = 'It wasn\'t turned on: $method was cancelled.';
      }
    });
    await refresh();
  }

  @override
  Widget build(BuildContext context) {
    final status = this.status;
    final method = (status?['method'] as String?) ?? 'Windows Hello';
    final Widget unlock;
    if (status == null) {
      unlock = const Padding(
        padding: EdgeInsets.all(16),
        child: Center(
          child: SizedBox(
            width: 20,
            height: 20,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
        ),
      );
    } else if (status['status'] == 'setUp' || status['status'] == 'notSetUp') {
      unlock = SwitchListTile(
        key: const ValueKey('biometric'),
        value: status['status'] == 'setUp',
        onChanged: working ? null : (on) => turn(on, method),
        secondary: const Icon(Icons.fingerprint_rounded),
        title: Text('Unlock with $method'),
        subtitle: Text(
          isPhone
              ? 'Instead of your master password'
              : 'Also in your browser extension, which shares it.',
        ),
        contentPadding: EdgeInsets.zero,
      );
    } else {
      unlock = ListTile(
        leading: Icon(Icons.fingerprint_rounded, color: context.colors.muted),
        title: Text('Unlock with $method'),
        subtitle: Text(
          status['status'] == 'noAgent'
              ? 'Needs PassBundle\'s desktop agent, which isn\'t running.'
              : '${method[0].toUpperCase()}${method.substring(1)} isn\'t set up on this ${isPhone ? 'phone' : 'computer'}.',
        ),
        contentPadding: EdgeInsets.zero,
      );
    }
    final sleepNote = [
      if (!isPhone) 'Or when its screen locks',
      if (policy['onSystemLock'] == true) 'Your organization keeps this on',
    ].join('\n');
    return AlertDialog(
      title: const Text('Settings'),
      // Scrolls on a small phone, or with the keyboard up.
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              unlock,
              const Divider(height: 24),
              lockAfter(),
              SwitchListTile(
                value:
                    preferences['lockOnSleep'] == true ||
                    policy['onSystemLock'] == true,
                onChanged: policy['onSystemLock'] == true
                    ? null
                    : (on) => choose({'lockOnSleep': on}),
                secondary: const Icon(Icons.bedtime_outlined),
                title: Text(
                  isPhone
                      ? 'Lock when the phone locks'
                      : 'Lock when the computer sleeps',
                ),
                subtitle: sleepNote.isEmpty ? null : Text(sleepNote),
                contentPadding: EdgeInsets.zero,
              ),
              if (note != null) ...[
                const SizedBox(height: 8),
                Text(
                  note!,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
              ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Close'),
        ),
      ],
    );
  }
}

/// The gear that opens Settings.
class SettingsButton extends StatelessWidget {
  const SettingsButton({super.key, this.color});

  final Color? color;

  @override
  Widget build(BuildContext context) => IconButton(
    tooltip: 'Settings',
    icon: Icon(Icons.settings_outlined, color: color),
    onPressed: () => showDialog<void>(
      context: context,
      builder: (_) => const SettingsDialog(),
    ),
  );
}
