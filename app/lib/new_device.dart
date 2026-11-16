// Signing in on a device the account has never used (DESIGN.md in the meta
// repo, "New-device check"): the server holds the sign-in until the person
// types the number this screen shows into a device where they're already
// signed in, or gives a code sent by email. The core asks how it stands
// every 3 seconds and finishes the sign-in once it's approved
// (core/src/new_device.rs); this screen shows the number, what a code by
// email can do, and what happened. No key moves either way.
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'history.dart' show whenSaved;
import 'theme.dart';
import 'widgets.dart';

/// What the core does for the waiting screen. Tests stand in for it.
class NewDeviceCalls {
  const NewDeviceCalls();

  /// How it stands now: `newDevice` with its `check`, or `done`.
  Future<Answer> status() => Core.newDeviceStatus();

  /// "Email me a code", now or after the day's wait.
  Future<Answer> email() => Core.newDeviceEmail();

  /// The code from the email.
  Future<Answer> code(String code) => Core.newDeviceCode(code);
}

/// How long a code by email waits when other devices could approve.
const emailWaitHours = 24;

/// The note beside "Email me a code": what it costs.
const emailIsWeaker =
    'A code by email is less safe than approving from your own device: anyone who can read your email could use it. Two-step login is safer than both.';

class NewDeviceScreen extends StatefulWidget {
  const NewDeviceScreen({
    super.key,
    required this.answer,
    required this.onSignedIn,
    required this.onStartOver,
    this.onTwoStep,
    this.calls = const NewDeviceCalls(),
    this.every = const Duration(seconds: 3),
  });

  /// The core's answer holding the sign-in: `check` (core/src/ffi.rs,
  /// `new_device_check`).
  final Map<String, dynamic> answer;
  final VoidCallback onSignedIn;
  final VoidCallback onStartOver;

  /// The server asks for two-step login instead (it was turned on
  /// meanwhile): its answer, for the two-step screen.
  final ValueChanged<Map<String, dynamic>>? onTwoStep;
  final NewDeviceCalls calls;

  /// How often to ask how it stands.
  final Duration every;

  @override
  State<NewDeviceScreen> createState() => _NewDeviceScreenState();
}

class _NewDeviceScreenState extends State<NewDeviceScreen> {
  late Map<String, dynamic> check = Map<String, dynamic>.from(
    widget.answer['check'] as Map? ?? const {},
  );
  late final Timer ticker = Timer.periodic(widget.every, (_) => _ask());
  final code = TextEditingController();

  /// Asking how it stands is out.
  bool asking = false;

  /// The person pressed a button, and its call is out: asking how it
  /// stands waits, and an answer it was already waiting for is dropped.
  bool pressed = false;

  /// "Can't use your other devices?" was opened.
  bool explaining = false;
  String? error;

  String get status => check['status'] as String? ?? 'waiting';
  String get email => check['email'] as String? ?? 'never';
  bool get codeSent => check['codeSent'] == true;
  bool get waiting => status == 'waiting' || status == 'approved';

  @override
  void initState() {
    super.initState();
    ticker;
  }

  @override
  void dispose() {
    ticker.cancel();
    code.dispose();
    super.dispose();
  }

  /// Where an answer got to: signed in, two-step login, or the check as it
  /// stands now. True when this screen still shows it.
  bool _after(Answer answer) {
    if (!mounted) return false;
    if (answer.ok && answer.body['step'] == 'done') {
      ticker.cancel();
      widget.onSignedIn();
      return false;
    }
    if (answer.ok && answer.body['step'] == 'twoFactor') {
      ticker.cancel();
      widget.onTwoStep?.call(answer.body);
      return false;
    }
    if (answer.ok && answer.body['check'] is Map) {
      setState(() {
        check = Map<String, dynamic>.from(answer.body['check'] as Map);
        error = null;
      });
      if (!waiting) ticker.cancel();
      return true;
    }
    if (answer.signedOut) {
      // The core gave this sign-in up (another one started).
      ticker.cancel();
      setState(() => check = {...check, 'status': 'ended'});
      return true;
    }
    setState(() => error = answer.error);
    return true;
  }

  /// How it stands now, every few seconds while it waits.
  Future<void> _ask() async {
    if (asking || pressed || !waiting || !mounted) return;
    asking = true;
    try {
      final answer = await whileBusy(widget.calls.status);
      // A button pressed meanwhile has the newer answer, unless this one
      // signed in.
      if (pressed && answer.body['step'] != 'done') return;
      // A network hiccup isn't the end: it asks again in a moment.
      if (!answer.ok && answer.offline) {
        if (mounted) {
          setState(
            () => error =
                'Couldn\'t reach your server. PassBundle keeps trying.',
          );
        }
        return;
      }
      _after(answer);
    } finally {
      asking = false;
    }
  }

  /// A button's call, one at a time.
  Future<void> _press(Future<Answer> Function() call) async {
    if (pressed) return;
    setState(() {
      pressed = true;
      error = null;
    });
    try {
      final answer = await whileBusy(call);
      if (_after(answer) && !answer.ok) code.clear();
    } finally {
      if (mounted) setState(() => pressed = false);
    }
  }

  Future<void> _sendCode() => _press(() => widget.calls.code(code.text));

  Widget _number(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Center(
      child: Container(
        key: const ValueKey('new-device-number'),
        padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 12),
        decoration: BoxDecoration(
          color: scheme.primary.withValues(alpha: 0.08),
          borderRadius: BorderRadius.circular(12),
        ),
        child: Text(
          check['number'] as String? ?? '',
          // Read as two digits, one at a time, by a screen reader.
          semanticsLabel: (check['number'] as String? ?? '').split('').join(' '),
          style: const TextStyle(
            fontSize: 56,
            fontWeight: FontWeight.w700,
            fontVariations: [FontVariation('wght', 700)],
            letterSpacing: 6,
            height: 1.1,
          ),
        ),
      ),
    );
  }

  Widget _muted(BuildContext context, String text) => Text(
    text,
    style: TextStyle(color: context.colors.muted, fontSize: 13),
  );

  /// What a code by email can do now.
  List<Widget> _email(BuildContext context) {
    if (codeSent) {
      final to = check['emailTo'] as String?;
      return [
        Text(
          to == null
              ? 'Your server emailed you a code.'
              : 'Your server emailed a code to $to.',
        ),
        const SizedBox(height: 12),
        TextField(
          key: const ValueKey('new-device-code'),
          controller: code,
          enabled: !pressed,
          keyboardType: TextInputType.number,
          autocorrect: false,
          enableSuggestions: false,
          autofillHints: const [AutofillHints.oneTimeCode],
          inputFormatters: [
            FilteringTextInputFormatter.allow(RegExp(r'[0-9 -]')),
          ],
          onSubmitted: (_) => _sendCode(),
          decoration: InputDecoration(
            labelText: 'Code from the email',
            errorText: error,
            errorMaxLines: 4,
          ),
        ),
        const SizedBox(height: 12),
        WorkingButton(label: 'Continue', working: pressed, onPressed: _sendCode),
        const SizedBox(height: 4),
        TextButton(
          onPressed: pressed ? null : () => _press(widget.calls.email),
          child: const Text('Send another code'),
        ),
      ];
    }
    switch (email) {
      case 'now':
        return [
          OutlinedButton(
            onPressed: pressed ? null : () => _press(widget.calls.email),
            child: const Text('Email me a code'),
          ),
          const SizedBox(height: 8),
          _muted(context, emailIsWeaker),
        ];
      case 'later':
        return [
          TextButton(
            onPressed: pressed
                ? null
                : () => setState(() => explaining = !explaining),
            child: const Text('Can\'t use your other devices?'),
          ),
          if (explaining) ...[
            const SizedBox(height: 4),
            const Text(
              'If you can\'t reach any device where you\'re signed in, PassBundle can email you a code instead, after a $emailWaitHours-hour wait. Your email and your devices hear about it right away, and any of them can deny this sign-in meanwhile.',
            ),
            const SizedBox(height: 8),
            _muted(context, emailIsWeaker),
            const SizedBox(height: 12),
            OutlinedButton(
              onPressed: pressed ? null : () => _press(widget.calls.email),
              child: const Text('Email me a code in $emailWaitHours hours'),
            ),
          ],
        ];
      case 'waiting':
        final after = check['emailAfter'] as String?;
        return [
          Text(
            after == null
                ? 'You asked for a code by email. Once the wait is over, sign in again on this device to get it.'
                : 'You asked for a code by email. After ${whenSaved(after)}, sign in again on this device to get it.',
          ),
          const SizedBox(height: 8),
          _muted(
            context,
            'Your other devices can still approve or deny this sign-in meanwhile.',
          ),
        ];
      default:
        return const [];
    }
  }

  Widget _error(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 12),
    child: Text(
      error!,
      style: TextStyle(color: Theme.of(context).colorScheme.error),
    ),
  );

  @override
  Widget build(BuildContext context) {
    if (!waiting) {
      final denied = status == 'denied';
      return Centered(
        children: [
          heading(
            context,
            denied ? 'Sign-in denied' : 'This sign-in ended',
            denied
                ? (check['deniedWhy'] == 'number'
                      ? 'The number typed on your other device didn\'t match, so this sign-in was denied.'
                      : 'This sign-in was denied from another of your devices.')
                : 'It wasn\'t approved in time.',
          ),
          Text(
            denied
                ? 'If it was you, start over to get a new number.'
                : 'Start over to get a new number.',
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 16),
          FilledButton(
            onPressed: widget.onStartOver,
            child: const Text('Start over'),
          ),
        ],
      );
    }
    return Centered(
      children: [
        heading(
          context,
          'Approve this sign-in',
          'This device is new to your account.',
        ),
        const Text(
          'On a device where you\'re already signed in, open your vault and type this number:',
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 16),
        _number(context),
        const SizedBox(height: 16),
        Text(
          email == 'waiting'
              ? 'Waiting for approval.'
              : 'Waiting for approval. This works for 10 minutes.',
          textAlign: TextAlign.center,
          style: TextStyle(color: context.colors.muted),
        ),
        const SizedBox(height: 20),
        ..._email(context),
        // Under the code field, its own error shows there.
        if (error != null && !codeSent) _error(context),
        const SizedBox(height: 8),
        TextButton(
          onPressed: pressed ? null : widget.onStartOver,
          child: const Text('Start over'),
        ),
      ],
    );
  }
}
