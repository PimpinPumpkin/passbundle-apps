// Settings: unlocking with Windows Hello or Touch ID, through the desktop
// agent and shared with the browser extension (docs/core.md, "Unlock with
// Windows Hello or Touch ID"), and when the app locks by itself ("Locking by
// itself"), within what an organization's policy allows ("Locking by
// policy"). On Android and the Mac, updates from PassBundle's releases
// (updates.dart).
import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'unlock_offer.dart';
import 'updates.dart';
import 'widgets.dart';

/// The waits offered, in minutes: -1 is "when the app closes", and on phones
/// 0 is "Immediately" (when the app leaves the screen).
List<int> get lockChoices => [if (isPhone) 0, 1, 5, 15, 30, 60, 240, -1];

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

String themeName(ThemeMode mode) => switch (mode) {
  ThemeMode.system => 'Follow the system',
  ThemeMode.light => 'Light',
  ThemeMode.dark => 'Dark',
};

/// Every setting, as groups in a column: a heading, rows on a card, and what
/// a policy does under them. On a phone (or a phone-wide window) they're a tab
/// of their own beside the vault ([SettingsPage]); on a computer, a dialog
/// from the sidebar's gear ([SettingsDialog]), which scrolls them.
class SettingsList extends StatefulWidget {
  const SettingsList({
    super.key,
    this.email,
    this.server,
    this.mode,
    this.onMode,
  });

  /// Who's signed in, and where, shown on a phone (a computer's sidebar has it).
  final String? email;
  final String? server;

  /// Light, dark, or following the system, where the screen has no menu for it.
  final ThemeMode? mode;
  final ValueChanged<ThemeMode>? onMode;

  @override
  State<SettingsList> createState() => _SettingsListState();
}

class _SettingsListState extends State<SettingsList> {
  final core = Core();
  late Map<String, dynamic> preferences = core.preferences();
  late final Map<String, dynamic> policy = core.lockPolicy();
  Map<String, dynamic>? status;

  /// Browsers trusted for Android's autofill (none elsewhere).
  List<Map<String, String>> browsers = const [];
  bool working = false;
  String? note;
  late ThemeMode? mode = widget.mode;

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

  @override
  void initState() {
    super.initState();
    refresh();
  }

  Future<void> refresh() async {
    final next = await Core.biometricStatus();
    final trusted = await Core.trustedBrowsers();
    if (mounted) {
      setState(() {
        status = next;
        browsers = trusted;
      });
    }
  }

  Future<void> untrust(String package) async {
    await Core.untrustBrowser(package);
    final trusted = await Core.trustedBrowsers();
    if (mounted) setState(() => browsers = trusted);
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
    final Answer answer;
    if (on) {
      // The biometric first; only then is the user key sealed behind it.
      answer = await turnOnBiometric();
    } else {
      for (var i = 0; i < 600 && busy.value; i++) {
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      answer = await whileBusy(Core.removeBiometric);
      // Turned off on purpose: the vault doesn't offer it again for a while.
      if (answer.ok) declineOffer();
    }
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

  Widget unlock() {
    final status = this.status;
    final method = (status?['method'] as String?) ?? 'Windows Hello';
    if (status == null) {
      return const SizedBox(
        height: 72,
        child: Center(
          child: SizedBox(
            width: 20,
            height: 20,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
        ),
      );
    }
    if (status['status'] == 'setUp' || status['status'] == 'notSetUp') {
      return SwitchListTile(
        key: const ValueKey('biometric'),
        value: status['status'] == 'setUp',
        onChanged: working ? null : (on) => turn(on, method),
        title: Text('Unlock with $method'),
        subtitle: Text(
          isPhone
              ? 'Instead of your master password'
              : 'Also in your browser extension, which shares it',
        ),
      );
    }
    return ListTile(
      title: Text(
        'Unlock with $method',
        style: TextStyle(color: context.colors.muted),
      ),
      subtitle: Text(
        status['status'] == 'noAgent'
            ? 'Needs PassBundle\'s desktop agent, which isn\'t running'
            : '${method[0].toUpperCase()}${method.substring(1)} isn\'t set up on this ${isPhone ? 'phone' : 'computer'}',
      ),
    );
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
    return ListTile(
      title: const Text('Lock after'),
      subtitle: Text(lockAfterName(minutes)),
      enabled: choices.length > 1,
      onTap: () async {
        final m = await chooseOne<int>(
          context,
          title: 'Lock after',
          description: isPhone
              ? 'Away from PassBundle'
              : 'Without using the computer',
          selected: minutes,
          choices: [for (final m in choices) (m, lockAfterName(m))],
        );
        if (m != null && mounted) {
          choose({'lockAfterMinutes': m < 0 ? null : m});
        }
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final sleepNote = [
      if (!isPhone) 'Or when its screen locks',
      if (policy['onSystemLock'] == true) 'Your organization keeps this on',
    ].join('\n');
    final mode = this.mode;
    final onMode = widget.onMode;
    final email = widget.email;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SettingsGroup(
          title: 'Unlock',
          footer: note,
          footerColor: scheme.error,
          children: [unlock()],
        ),
        SettingsGroup(
          title: 'Locking',
          footer: policyWait(policy),
          children: [
            lockAfter(),
            SwitchListTile(
              value:
                  preferences['lockOnSleep'] == true ||
                  policy['onSystemLock'] == true,
              onChanged: policy['onSystemLock'] == true
                  ? null
                  : (on) => choose({'lockOnSleep': on}),
              title: Text(
                isPhone
                    ? 'Lock when the phone locks'
                    : 'Lock when the computer sleeps',
              ),
              subtitle: sleepNote.isEmpty ? null : Text(sleepNote),
            ),
          ],
        ),
        if (mode != null && onMode != null)
          SettingsGroup(
            title: 'Appearance',
            children: [
              ListTile(
                title: const Text('Theme'),
                subtitle: Text(themeName(mode)),
                onTap: () async {
                  final chosen = await chooseOne<ThemeMode>(
                    context,
                    title: 'Theme',
                    selected: mode,
                    choices: [
                      for (final m in ThemeMode.values) (m, themeName(m)),
                    ],
                  );
                  if (chosen == null || !mounted) return;
                  setState(() => this.mode = chosen);
                  onMode(chosen);
                },
              ),
            ],
          ),
        if (browsers.isNotEmpty)
          SettingsGroup(
            title: 'Browsers you trust for autofill',
            footer: "PassBundle fills a website's logins in these too.",
            children: [
              for (final b in browsers)
                ListTile(
                  title: Text(b['name']!),
                  subtitle: Text(
                    b['package']!,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                  trailing: TextButton(
                    onPressed: () => untrust(b['package']!),
                    child: const Text('Stop trusting'),
                  ),
                ),
            ],
          ),
        if (updates case final updates?) UpdatesSection(updates: updates),
        if (email != null)
          SettingsGroup(
            title: 'Account',
            children: [
              ListTile(
                title: Text(
                  email,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                subtitle: widget.server == null
                    ? null
                    : Text(
                        Uri.tryParse(widget.server!)?.host ?? widget.server!,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
              ),
            ],
          ),
        // About: the logo, and this version where the app knows it.
        Padding(
          padding: const EdgeInsets.fromLTRB(4, 32, 4, 8),
          child: Column(
            children: [
              const Wordmark(size: 28),
              if (updates?.version case final version?) ...[
                const SizedBox(height: 8),
                Text(
                  'Version $version',
                  style: TextStyle(color: context.colors.muted, fontSize: 13),
                ),
              ],
            ],
          ),
        ),
      ],
    );
  }
}

/// Settings as a tab of its own, on a phone.
class SettingsPage extends StatelessWidget {
  const SettingsPage({
    super.key,
    this.email,
    this.server,
    this.mode,
    this.onMode,
  });

  final String? email;
  final String? server;
  final ThemeMode? mode;
  final ValueChanged<ThemeMode>? onMode;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      const PageTitle('Settings'),
      Expanded(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(16, 0, 16, 32),
          children: [
            SettingsList(
              email: email,
              server: server,
              mode: mode,
              onMode: onMode,
            ),
          ],
        ),
      ),
    ],
  );
}

/// Settings over the vault, on a computer.
class SettingsDialog extends StatelessWidget {
  const SettingsDialog({super.key});

  @override
  Widget build(BuildContext context) => Dialog(
    backgroundColor: context.colors.ground,
    clipBehavior: Clip.antiAlias,
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 480, maxHeight: 760),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(24, 22, 24, 0),
            child: Text(
              'Settings',
              style: Theme.of(context).textTheme.headlineSmall,
            ),
          ),
          // Scrolls on a small screen, or with a large text size.
          const Flexible(
            child: SingleChildScrollView(
              padding: EdgeInsets.fromLTRB(20, 0, 20, 8),
              child: SettingsList(),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(24, 4, 16, 12),
            child: Align(
              alignment: Alignment.centerRight,
              child: TextButton(
                onPressed: () => Navigator.pop(context),
                child: const Text('Close'),
              ),
            ),
          ),
        ],
      ),
    ),
  );
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
