// Unlocking with a PIN (DESIGN.md in the meta repo, "Unlock with a PIN"):
// Settings' row for it, and the sheet that sets one up, which asks for the
// master password again. The lock screen's PIN field is in sign_in.dart.
// The PIN and the master password go to the core and their fields are
// cleared at once, as the lock screen's are; neither is kept here. Where the
// device can't keep a PIN sealed to it (Linux, a Mac without a Secure
// Enclave), the core says so and the row isn't shown.
import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

/// Settings' row, while an organization's policy forbids a PIN.
const pinPolicyReason = "Your organization doesn't allow unlocking with a PIN";

/// The biometric switch's, in a vault a PIN opened: a PIN can't be traded
/// up for biometrics.
const biometricsAfterPin = 'Unlock with your master password to turn this on';

/// Whether the core's [Core.pinStatus] offers a PIN here at all.
bool pinOffered(Map<String, dynamic>? status) =>
    const {'on', 'off', 'policy'}.contains(status?['status']);

/// A PIN's length as the core counts it: in characters, not UTF-16 units.
int pinLength(String pin) => pin.runes.length;

/// The sheet that sets the account's PIN: the master password, then the
/// new PIN twice. True once it's set.
Future<bool> setUpPin(
  BuildContext context,
  Map<String, dynamic> status,
) async =>
    await showDialog<bool>(
      context: context,
      builder: (_) => PinSetup(
        least: (status['minCharacters'] as num?)?.toInt() ?? 4,
        most: (status['maxCharacters'] as num?)?.toInt() ?? 128,
      ),
    ) ==
    true;

class PinSetup extends StatefulWidget {
  const PinSetup({super.key, this.least = 4, this.most = 128});

  /// The fewest and most characters a PIN may have.
  final int least;
  final int most;

  @override
  State<PinSetup> createState() => _PinSetupState();
}

class _PinSetupState extends State<PinSetup> {
  final password = TextEditingController();
  final pin = TextEditingController();
  final again = TextEditingController();
  bool working = false;
  String? pinError;
  String? againError;
  String? error;

  @override
  void dispose() {
    for (final c in [password, pin, again]) {
      c.clear();
      c.dispose();
    }
    super.dispose();
  }

  Future<void> save() async {
    final typedPin = pin.text;
    final length = pinLength(typedPin);
    final problem = length < widget.least
        ? 'A PIN needs at least ${widget.least} characters.'
        : length > widget.most
        ? 'A PIN can have at most ${widget.most} characters.'
        : null;
    final mismatch = problem == null && again.text != typedPin
        ? "The PINs don't match."
        : null;
    if (password.text.isEmpty || problem != null || mismatch != null) {
      setState(() {
        error = password.text.isEmpty ? 'Type your master password.' : null;
        pinError = problem;
        againError = mismatch;
      });
      if (mismatch != null) again.clear();
      return;
    }
    final typed = password.text;
    for (final c in [password, pin, again]) {
      c.clear();
    }
    setState(() {
      working = true;
      error = pinError = againError = null;
    });
    final answer = await whileBusy(() => Core.setPin(typed, typedPin));
    if (!mounted) return;
    if (answer.ok) {
      Navigator.of(context).pop(true);
      return;
    }
    setState(() {
      working = false;
      error = answer.error ?? "Couldn't set the PIN. Try again.";
    });
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return AlertDialog(
      title: const Text('Unlock with a PIN'),
      content: SizedBox(
        width: 380,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                'On this device only. After 5 wrong tries, it turns off and '
                'PassBundle asks for your master password.',
                style: TextStyle(color: context.colors.muted, fontSize: 14),
              ),
              const SizedBox(height: 16),
              _SecretField(
                key: const ValueKey('pin-master-password'),
                controller: password,
                label: 'Master password',
                enabled: !working,
                autofocus: true,
                onSubmitted: save,
              ),
              const SizedBox(height: 12),
              _SecretField(
                key: const ValueKey('pin-new'),
                controller: pin,
                label: 'New PIN',
                helper: 'At least ${widget.least} characters, any you like',
                error: pinError,
                enabled: !working,
                onSubmitted: save,
              ),
              const SizedBox(height: 12),
              _SecretField(
                key: const ValueKey('pin-again'),
                controller: again,
                label: 'New PIN again',
                error: againError,
                enabled: !working,
                onSubmitted: save,
              ),
              if (error != null) ...[
                const SizedBox(height: 12),
                Text(error!, style: TextStyle(color: scheme.error)),
              ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: working ? null : () => Navigator.of(context).pop(false),
          child: const Text('Cancel'),
        ),
        WorkingButton(label: 'Set PIN', working: working, onPressed: save),
      ],
    );
  }
}

/// A field for a master password or a PIN: hidden, with Show, and nothing
/// the keyboard would learn from it.
class _SecretField extends StatefulWidget {
  const _SecretField({
    super.key,
    required this.controller,
    required this.label,
    required this.onSubmitted,
    this.helper,
    this.error,
    this.enabled = true,
    this.autofocus = false,
  });

  final TextEditingController controller;
  final String label;
  final String? helper;
  final String? error;
  final bool enabled;
  final bool autofocus;
  final VoidCallback onSubmitted;

  @override
  State<_SecretField> createState() => _SecretFieldState();
}

class _SecretFieldState extends State<_SecretField> {
  bool hidden = true;

  @override
  Widget build(BuildContext context) => TextField(
    controller: widget.controller,
    obscureText: hidden,
    enabled: widget.enabled,
    autofocus: widget.autofocus,
    enableSuggestions: false,
    autocorrect: false,
    enableIMEPersonalizedLearning: false,
    onSubmitted: (_) => widget.onSubmitted(),
    decoration: InputDecoration(
      labelText: widget.label,
      helperText: widget.helper,
      errorText: widget.error,
      errorMaxLines: 3,
      suffixIcon: IconButton(
        tooltip: hidden ? 'Show' : 'Hide',
        icon: Icon(
          hidden ? Icons.visibility_outlined : Icons.visibility_off_outlined,
        ),
        onPressed: () => setState(() => hidden = !hidden),
      ),
    ),
  );
}
