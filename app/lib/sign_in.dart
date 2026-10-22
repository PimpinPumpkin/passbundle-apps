// Signing in (with two-step login), and unlocking the vault kept on this
// device. The master password goes to the core and the field is cleared at
// once (docs/core.md, "Memory"). People sign in to PassBundle's own server
// without being asked; a server an organization set by policy is used and
// shown; and someone hosting their own types it in, behind "Self-hosted?".
import 'dart:io';

import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

final _measuring = Platform.environment['PB_MEASURE'] == '1';

class SignInScreen extends StatefulWidget {
  const SignInScreen({
    super.key,
    this.server,
    this.email,
    required this.onSignedIn,
    this.onCancel,
  });

  final String? server;
  final String? email;
  final VoidCallback onSignedIn;

  /// Back to the vault kept on this device, when there is one.
  final VoidCallback? onCancel;

  @override
  State<SignInScreen> createState() => _SignInScreenState();
}

class _SignInScreenState extends State<SignInScreen> {
  final servers = Core().servers();
  late final ours = servers['ours'] as String;
  late final managed = servers['managed'] as String?;

  /// Someone's own server: shown when they ask, or when they signed in to one before.
  late bool selfHosted =
      managed == null && widget.server != null && widget.server != ours;
  late final server = TextEditingController(
    text: selfHosted ? widget.server : '',
  );

  String get signInTo => managed ?? (selfHosted ? server.text.trim() : ours);
  late final email = TextEditingController(text: widget.email ?? '');
  final password = TextEditingController();
  final code = TextEditingController();
  String? error;
  bool working = false;
  bool hidden = true;
  bool remember = false;

  /// The second step's methods, once the server asks for one: 0 authenticator, 1 email.
  List<int>? providers;
  int provider = 0;
  bool emailSent = false;

  Future<void> signIn() async {
    final typed = password.text;
    // The core has it now; the field doesn't keep it.
    password.clear();
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(
      () => Core.signIn(signInTo, email.text.trim(), typed),
    );
    if (!mounted) return;
    if (answer.ok && answer.body['step'] == 'twoFactor') {
      final offered = (answer.body['providers'] as List).cast<int>();
      final usable = offered.where((p) => p == 0 || p == 1).toList();
      setState(() {
        working = false;
        providers = usable;
        provider = usable.contains(0) ? 0 : (usable.isEmpty ? -1 : 1);
        error = usable.isEmpty
            ? 'This account uses a kind of two-step login this app can\'t do yet.'
            : null;
      });
    } else if (answer.ok) {
      widget.onSignedIn();
    } else {
      setState(() {
        working = false;
        error = answer.error;
      });
    }
  }

  Future<void> sendEmail() async {
    setState(() => working = true);
    final answer = await whileBusy(Core.sendEmailCode);
    if (!mounted) return;
    setState(() {
      working = false;
      emailSent = answer.ok;
      error = answer.ok ? null : answer.error;
    });
  }

  Future<void> finish() async {
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(
      () => Core.finishSignIn(provider, code.text.trim(), remember),
    );
    if (!mounted) return;
    if (answer.ok) {
      widget.onSignedIn();
    } else {
      code.clear();
      setState(() {
        working = false;
        error = answer.error;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final step = providers;
    if (step != null) return _secondStep(context, step);
    return Centered(
      children: [
        heading(
          context,
          'Sign in to PassBundle',
          managed == null
              ? null
              : 'Your organization\'s server: ${Uri.tryParse(managed!)?.host ?? managed}',
        ),
        if (selfHosted) ...[
          TextField(
            controller: server,
            enabled: !working,
            keyboardType: TextInputType.url,
            decoration: const InputDecoration(
              labelText: 'Server',
              hintText: 'https://vault.example.com',
            ),
          ),
          const SizedBox(height: 12),
        ],
        TextField(
          controller: email,
          enabled: !working,
          keyboardType: TextInputType.emailAddress,
          autofillHints: const [AutofillHints.email],
          decoration: const InputDecoration(labelText: 'Email'),
        ),
        const SizedBox(height: 12),
        TextField(
          controller: password,
          obscureText: hidden,
          enabled: !working,
          onSubmitted: (_) => signIn(),
          decoration: InputDecoration(
            labelText: 'Master password',
            errorText: error,
            errorMaxLines: 3,
            suffixIcon: IconButton(
              tooltip: hidden ? 'Show' : 'Hide',
              icon: Icon(
                hidden
                    ? Icons.visibility_outlined
                    : Icons.visibility_off_outlined,
              ),
              onPressed: () => setState(() => hidden = !hidden),
            ),
          ),
        ),
        const SizedBox(height: 16),
        WorkingButton(label: 'Sign in', working: working, onPressed: signIn),
        if (managed == null) ...[
          const SizedBox(height: 8),
          TextButton(
            onPressed: working
                ? null
                : () => setState(() => selfHosted = !selfHosted),
            child: Text(
              selfHosted ? 'Use PassBundle\'s server' : 'Self-hosted?',
            ),
          ),
        ],
        if (widget.onCancel != null) ...[
          const SizedBox(height: 8),
          TextButton(
            onPressed: working ? null : widget.onCancel,
            child: const Text('Back to my vault'),
          ),
        ],
      ],
    );
  }

  Widget _secondStep(BuildContext context, List<int> step) {
    final viaEmail = provider == 1;
    return Centered(
      children: [
        heading(
          context,
          'Two-step login',
          viaEmail
              ? (emailSent ? 'We emailed you a code.' : 'Get a code by email.')
              : 'Enter the code from your authenticator app.',
        ),
        if (step.length > 1)
          SegmentedButton<int>(
            segments: const [
              ButtonSegment(value: 0, label: Text('Authenticator app')),
              ButtonSegment(value: 1, label: Text('Email')),
            ],
            selected: {provider},
            onSelectionChanged: (s) => setState(() => provider = s.first),
          ),
        if (step.length > 1) const SizedBox(height: 16),
        if (viaEmail && !emailSent)
          WorkingButton(
            label: 'Email me a code',
            working: working,
            onPressed: sendEmail,
          )
        else ...[
          TextField(
            controller: code,
            enabled: !working && provider >= 0,
            autofocus: true,
            keyboardType: TextInputType.number,
            autofillHints: const [AutofillHints.oneTimeCode],
            onSubmitted: (_) => finish(),
            decoration: InputDecoration(
              labelText: 'Code',
              errorText: error,
              errorMaxLines: 3,
            ),
          ),
          const SizedBox(height: 8),
          CheckboxListTile(
            value: remember,
            onChanged: working
                ? null
                : (v) => setState(() => remember = v ?? false),
            title: const Text('Don\'t ask again on this device for a while'),
            controlAffinity: ListTileControlAffinity.leading,
            contentPadding: EdgeInsets.zero,
          ),
          const SizedBox(height: 8),
          WorkingButton(label: 'Continue', working: working, onPressed: finish),
        ],
        if (error != null && viaEmail && !emailSent) ...[
          const SizedBox(height: 12),
          Text(
            error!,
            style: TextStyle(color: Theme.of(context).colorScheme.error),
          ),
        ],
        const SizedBox(height: 8),
        TextButton(
          onPressed: working
              ? null
              : () => setState(() {
                  providers = null;
                  error = null;
                  emailSent = false;
                }),
          child: const Text('Start over'),
        ),
      ],
    );
  }
}

class UnlockScreen extends StatefulWidget {
  const UnlockScreen({
    super.key,
    required this.email,
    required this.server,
    required this.onUnlocked,
    required this.onSignOut,
  });

  final String email;
  final String server;
  final ValueChanged<int> onUnlocked;
  final VoidCallback onSignOut;

  @override
  State<UnlockScreen> createState() => _UnlockScreenState();
}

class _UnlockScreenState extends State<UnlockScreen> {
  final password = TextEditingController(
    text: _measuring ? (Platform.environment['PB_MEASURE_PASSWORD'] ?? '') : '',
  );
  String? error;
  bool working = false;
  bool hidden = true;

  /// "Windows Hello" or "Touch ID" when this account can unlock with it here.
  String? method;

  @override
  void initState() {
    super.initState();
    if (_measuring) {
      WidgetsBinding.instance.addPostFrameCallback((_) => unlock());
    } else {
      Core.biometricStatus().then((status) {
        if (mounted && status['status'] == 'setUp') {
          setState(() => method = status['method'] as String);
        }
      });
    }
  }

  Future<void> unlockWithBiometric() async {
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(Core.unlockWithBiometric);
    if (!mounted) return;
    if (answer.ok && answer.body['unlocked'] == true) {
      widget.onUnlocked(0);
      return;
    }
    setState(() {
      working = false;
      error = answer.ok ? null : answer.error;
      // An out-of-date setup was turned off: the master password, then turn it on again.
      if (!answer.ok) method = null;
    });
  }

  Future<void> unlock() async {
    final typed = password.text;
    password.clear();
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(() => Core.unlock(typed));
    if (!mounted) return;
    if (answer.ok) {
      widget.onUnlocked((answer.body['ms'] as num).toInt());
    } else {
      setState(() {
        working = false;
        error = answer.error;
      });
    }
  }

  @override
  Widget build(BuildContext context) => Centered(
    children: [
      heading(context, 'Unlock PassBundle', widget.email),
      TextField(
        controller: password,
        obscureText: hidden,
        autofocus: true,
        enabled: !working,
        onSubmitted: (_) => unlock(),
        decoration: InputDecoration(
          labelText: 'Master password',
          errorText: error,
          suffixIcon: IconButton(
            tooltip: hidden ? 'Show' : 'Hide',
            icon: Icon(
              hidden
                  ? Icons.visibility_outlined
                  : Icons.visibility_off_outlined,
            ),
            onPressed: () => setState(() => hidden = !hidden),
          ),
        ),
      ),
      const SizedBox(height: 16),
      WorkingButton(label: 'Unlock', working: working, onPressed: unlock),
      if (method != null) ...[
        const SizedBox(height: 10),
        OutlinedButton.icon(
          onPressed: working ? null : unlockWithBiometric,
          icon: const Icon(Icons.fingerprint_rounded),
          label: Text('Unlock with $method'),
        ),
      ],
      const SizedBox(height: 18),
      Text(
        Uri.tryParse(widget.server)?.host ?? widget.server,
        textAlign: TextAlign.center,
        style: TextStyle(color: context.colors.muted, fontSize: 12),
      ),
      TextButton(
        onPressed: working
            ? null
            : () async {
                final sure = await showDialog<bool>(
                  context: context,
                  builder: (context) => AlertDialog(
                    title: const Text('Sign out?'),
                    content: Text(
                      method == null
                          ? 'This removes the vault from this device. Signing in again brings it back from the server.'
                          : isPhone
                          ? 'This removes the vault from this phone and turns off unlocking with $method. Signing in again brings the vault back from the server.'
                          : 'This removes the vault from this device and turns off unlocking with $method, in your browser extension too. Signing in again brings the vault back from the server.',
                    ),
                    actions: [
                      TextButton(
                        onPressed: () => Navigator.pop(context, false),
                        child: const Text('Cancel'),
                      ),
                      FilledButton(
                        onPressed: () => Navigator.pop(context, true),
                        child: const Text('Sign out'),
                      ),
                    ],
                  ),
                );
                if (sure == true) widget.onSignOut();
              },
        child: const Text('Sign out'),
      ),
    ],
  );
}
