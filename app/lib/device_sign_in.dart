// Signing in with a device (DESIGN.md in the meta repo, "Approving a sign-in
// from another device", "Our apps asking"): on a device that has signed in
// to the account before, the person picks "Sign in with a device" on the
// sign-in screen instead of typing the master password, and approves it on
// another of their devices. The core makes the key pair and the access code,
// sends the request, asks for the answer, opens the user key, and signs in
// (core/src/device_signin.rs); this screen shows the fingerprint phrase
// first and largest, asks the core how it stands every 5 seconds, and says
// what happened. Resend sends a new request in this one's place, with a new
// phrase; "Use master password instead" gives it up, and its key with it.
import 'dart:async';

import 'package:flutter/material.dart';

import 'core.dart';
import 'sign_in_requests.dart' show timeLeft;
import 'theme.dart';
import 'widgets.dart';

/// What the core does for signing in with a device. Tests stand in for it.
class DeviceSignInCalls {
  const DeviceSignInCalls();

  /// Asks to sign in with a device (or again, in the last one's place):
  /// `deviceSignIn` with its `request`, or why not.
  Future<Answer> start(String server, String email) =>
      Core.signInWithDevice(server, email);

  /// How the request stands now: `deviceSignIn` with its `request`,
  /// `twoFactor`, or `done`.
  Future<Answer> status() => Core.deviceSignInStatus();

  /// Gives the request up, and its key with it.
  Future<void> giveUp() => Core.giveUpDeviceSignIn();
}

/// Tests only: the core's calls for signing in with a device, in place of
/// the core's own. Null is the core's.
@visibleForTesting
DeviceSignInCalls? debugDeviceSignIn;

/// The calls the screens use.
DeviceSignInCalls get deviceSignInCalls =>
    debugDeviceSignIn ?? const DeviceSignInCalls();

class DeviceSignInScreen extends StatefulWidget {
  const DeviceSignInScreen({
    super.key,
    required this.server,
    required this.answer,
    required this.onSignedIn,
    required this.onTwoStep,
    required this.onPassword,
    this.calls,
    this.every = const Duration(seconds: 5),
  });

  /// The server the request went to, for Resend.
  final String server;

  /// The core's answer that made the request: `request` (core/src/ffi.rs,
  /// `device_sign_in`).
  final Map<String, dynamic> answer;
  final VoidCallback onSignedIn;

  /// Two-step login follows the approval: its answer, for the two-step
  /// screen.
  final ValueChanged<Map<String, dynamic>> onTwoStep;

  /// Back to the master password, with what to say under it (null for
  /// nothing): the person chose it, or the server refused the sign-in for a
  /// reason a password sign-in would hear too (an organization's rules).
  final ValueChanged<String?> onPassword;

  /// The core's ([deviceSignInCalls]) unless a test gives its own.
  final DeviceSignInCalls? calls;

  /// How often to ask how it stands.
  final Duration every;

  @override
  State<DeviceSignInScreen> createState() => _DeviceSignInScreenState();
}

class _DeviceSignInScreenState extends State<DeviceSignInScreen> {
  DeviceSignInCalls get calls => widget.calls ?? deviceSignInCalls;

  late Map<String, dynamic> request = Map<String, dynamic>.from(
    widget.answer['request'] as Map? ?? const {},
  );

  Timer? ticker;

  /// Asking how it stands is out.
  bool asking = false;

  /// Resend's call is out: asking how it stands waits, and an answer it was
  /// already waiting for is dropped.
  bool pressed = false;

  /// What went wrong, said on the screen: a moment offline, or a request
  /// that couldn't be sent.
  String? error;

  /// The request ended with nothing the core can say about it (Resend
  /// couldn't send a new one, and the old one is gone).
  bool gone = false;

  String get status => request['status'] as String? ?? 'waiting';
  String get email => request['email'] as String? ?? '';
  String get phrase => request['fingerprint'] as String? ?? '';
  bool get waiting => status == 'waiting' && !gone;

  @override
  void initState() {
    super.initState();
    _tick();
  }

  void _tick() {
    ticker?.cancel();
    ticker = waiting ? Timer.periodic(widget.every, (_) => _ask()) : null;
  }

  @override
  void dispose() {
    ticker?.cancel();
    super.dispose();
  }

  /// Where an answer got to: signed in, two-step login, the request as it
  /// stands now, or what went wrong.
  void _after(Answer answer) {
    if (!mounted) return;
    final step = answer.body['step'];
    if (answer.ok && step == 'done') {
      ticker?.cancel();
      widget.onSignedIn();
      return;
    }
    if (answer.ok && step == 'twoFactor') {
      ticker?.cancel();
      widget.onTwoStep(answer.body);
      return;
    }
    if (answer.ok && answer.body['request'] is Map) {
      setState(() {
        request = Map<String, dynamic>.from(answer.body['request'] as Map);
        error = null;
        gone = false;
      });
      _tick();
      return;
    }
    if (answer.offline && answer.body['asking'] == true) {
      setState(
        () => error = 'Couldn\'t reach your server. PassBundle keeps trying.',
      );
      return;
    }
    if (answer.body['asking'] == true) {
      // A problem of the server's: the request still waits.
      setState(() => error = answer.error);
      return;
    }
    ticker?.cancel();
    if (answer.signedOut) {
      // The core gave this request up: another sign-in started.
      widget.onPassword(null);
      return;
    }
    // The server refused the sign-in itself (an organization's rules): said
    // under the master password, as a password sign-in hears it.
    widget.onPassword(answer.error);
  }

  /// How it stands now, every few seconds while it waits.
  Future<void> _ask() async {
    if (asking || pressed || !waiting || !mounted) return;
    asking = true;
    try {
      final answer = await whileBusy(calls.status);
      // Resend pressed meanwhile has the newer request, unless this one
      // signed in.
      if (pressed && answer.body['step'] != 'done') return;
      _after(answer);
    } finally {
      asking = false;
    }
  }

  /// A new request in this one's place, with a new phrase.
  Future<void> _resend() async {
    if (pressed) return;
    setState(() {
      pressed = true;
      error = null;
    });
    try {
      final answer = await whileBusy(() => calls.start(widget.server, email));
      if (!mounted) return;
      if (answer.ok) {
        _after(answer);
      } else {
        // Not sent. The last one may still wait (`asking`), or be gone.
        setState(() {
          error = answer.error;
          gone = answer.body['asking'] != true;
        });
        _tick();
      }
    } finally {
      if (mounted) setState(() => pressed = false);
    }
  }

  /// Back to the master password: the request is given up here.
  void _usePassword() {
    ticker?.cancel();
    calls.giveUp();
    widget.onPassword(null);
  }

  /// The phrase, first and largest: what the person compares with the one
  /// on the approving device.
  Widget _phrase(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          'Fingerprint phrase',
          style: TextStyle(color: context.colors.muted, fontSize: 13),
        ),
        const SizedBox(height: 4),
        Container(
          key: const ValueKey('device-sign-in-phrase'),
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
          decoration: BoxDecoration(
            color: scheme.primary.withValues(alpha: 0.08),
            borderRadius: BorderRadius.circular(12),
          ),
          child: Text(
            phrase,
            textAlign: TextAlign.center,
            style: const TextStyle(
              fontSize: 28,
              fontWeight: FontWeight.w700,
              fontVariations: [FontVariation('wght', 700)],
              height: 1.3,
            ),
          ),
        ),
      ],
    );
  }

  Widget _error(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 12),
    child: Text(
      error!,
      key: const ValueKey('device-sign-in-error'),
      textAlign: TextAlign.center,
      style: TextStyle(color: Theme.of(context).colorScheme.error),
    ),
  );

  Widget _usePasswordButton() => TextButton(
    key: const ValueKey('device-sign-in-password'),
    onPressed: _usePassword,
    child: const Text('Use master password instead'),
  );

  /// The heading and what to do, once a request has ended.
  (String, String) get _ended => switch (status) {
    'denied' => (
      'Sign-in denied',
      'This sign-in was denied on another of your devices. If this was you, try to sign in with the device again.',
    ),
    'expired' => (
      'This request expired',
      'It wasn\'t approved within 15 minutes. Send a new request, or use your master password.',
    ),
    'badKey' => (
      'This approval didn\'t work',
      'The approval didn\'t carry a key that opens your vault. Send a new request, or use your master password.',
    ),
    'unusable' => (
      'This approval can\'t be used here',
      'Your server didn\'t sign in with it. It may have been used already, or this device\'s network changed since it asked. Send a new request, or use your master password.',
    ),
    _ => (
      'This request ended',
      'Send a new request, or use your master password.',
    ),
  };

  @override
  Widget build(BuildContext context) {
    if (!waiting) {
      final (title, words) = _ended;
      return Centered(
        children: [
          heading(context, title, email.isEmpty ? null : email),
          Text(words, textAlign: TextAlign.center),
          if (error != null) _error(context),
          const SizedBox(height: 20),
          WorkingButton(
            key: const ValueKey('device-sign-in-resend'),
            label: 'Send a new request',
            working: pressed,
            onPressed: _resend,
          ),
          const SizedBox(height: 4),
          _usePasswordButton(),
        ],
      );
    }
    final left = Duration(
      seconds: (request['secondsLeft'] as num?)?.toInt() ?? 0,
    );
    return Centered(
      children: [
        heading(context, 'Sign in with a device', email),
        _phrase(context),
        const SizedBox(height: 16),
        const Text(
          'Open PassBundle on another device where you\'re signed in, or '
          'the web vault, and approve this sign-in there. Approve it only '
          'if that device shows this same phrase.',
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 12),
        Text(
          left > Duration.zero
              ? 'Waiting for approval. Expires in ${timeLeft(left).toLowerCase()}.'
              : 'Waiting for approval.',
          key: const ValueKey('device-sign-in-waiting'),
          textAlign: TextAlign.center,
          style: TextStyle(color: context.colors.muted),
        ),
        if (error != null) _error(context),
        const SizedBox(height: 20),
        OutlinedButton(
          key: const ValueKey('device-sign-in-resend'),
          onPressed: pressed ? null : _resend,
          child: pressed
              ? const SizedBox(
                  width: 18,
                  height: 18,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Text('Resend'),
        ),
        const SizedBox(height: 4),
        _usePasswordButton(),
      ],
    );
  }
}
