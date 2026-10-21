// Settings: unlocking with Windows Hello or Touch ID, through the desktop
// agent and shared with the browser extension (docs/core.md, "Unlock with
// Windows Hello or Touch ID").
import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

class SettingsDialog extends StatefulWidget {
  const SettingsDialog({super.key});

  @override
  State<SettingsDialog> createState() => _SettingsDialogState();
}

class _SettingsDialogState extends State<SettingsDialog> {
  Map<String, dynamic>? status;
  bool working = false;
  String? note;

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
          content: const Text('It turns off in your browser extension too.'),
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
        value: status['status'] == 'setUp',
        onChanged: working ? null : (on) => turn(on, method),
        secondary: const Icon(Icons.fingerprint_rounded),
        title: Text('Unlock with $method'),
        subtitle: const Text(
          'Also in your browser extension, which shares it.',
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
              : '$method isn\'t set up on this computer.',
        ),
        contentPadding: EdgeInsets.zero,
      );
    }
    return AlertDialog(
      title: const Text('Settings'),
      content: SizedBox(
        width: 420,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            unlock,
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
