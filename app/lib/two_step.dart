// Two-step login's second step, with every method the server offers that
// this device can do (DESIGN.md in the meta repo, "Two-step login in the
// apps"): a security key, Duo, a YubiKey's code, an authenticator app's code,
// an emailed code, and a recovery code. The core checks and sends each
// (core/src/two_step.rs, core/src/security_key); this screen picks the
// strongest one offered, lists the others, and says what happened.
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

/// The methods, in the order the screen suggests them, as the server numbers them.
enum Method {
  securityKey(7, 'Security key'),
  duo(2, 'Duo'),
  yubikey(3, 'YubiKey'),
  authenticator(0, 'Authenticator app'),
  email(1, 'Email'),
  recoveryCode(8, 'Recovery code');

  const Method(this.provider, this.label);

  final int provider;
  final String label;
}

/// The methods the server offers that the apps do, in the order the screen
/// suggests them. A recovery code always works besides, so it isn't listed.
List<Method> methodsOffered(Map<String, dynamic> offer) {
  final providers = (offer['providers'] as List? ?? const []).cast<int>();
  return [
    for (final method in Method.values)
      if (method != Method.recoveryCode && providers.contains(method.provider))
        method,
  ];
}

/// What the core does for each method. Tests stand in for it.
class TwoStepCalls {
  const TwoStepCalls();

  Future<Answer> finish(Method method, String code, bool remember) =>
      Core.finishSignIn(method.provider, code, remember);

  Future<Answer> sendEmail() => Core.sendEmailCode();

  Future<Answer> securityKey(bool remember) => Core.securityKeySignIn(remember);

  void cancelSecurityKey() => Core().securityKeyCancel();

  Future<Answer> duoStart() => Core.duoStart();

  /// Duo in the browser: the link it comes back with, or null if the
  /// person gave up ([cancelDuo]).
  Future<String?> duoBrowse(String url) => DuoBrowser.open(url);

  void cancelDuo() => DuoBrowser.cancel();

  Future<Answer> duoFinish(String link, bool remember) =>
      Core.duoFinish(link, remember);

  /// On Android, a YubiKey held to the phone gives its code over NFC:
  /// [onCode] hears it while on.
  void yubikeyNfc(bool on, void Function(String code)? onCode) =>
      YubikeyNfc.listen(on, onCode);
}

/// Duo's Universal Prompt in the browser, and its way back: on the Mac and
/// iPhones, Safari's sign-in sheet hands back the link itself; on Android,
/// the link opens the app again (LinkActivity.kt); on Windows and Linux,
/// the OS opens a second copy of the app with it, which leaves it for this
/// one (main.dart, core/src/two_step.rs).
class DuoBrowser {
  static Completer<String?>? _waiting;
  static Timer? _polling;
  static const links = MethodChannel('passbundle/links');

  static Future<String?> open(String url) async {
    cancel();
    if (Platform.isMacOS || Platform.isIOS) {
      return Core.platform.invokeMethod<String>('duoSignIn', {'url': url});
    }
    final waiting = _waiting = Completer<String?>();
    if (Platform.isAndroid) {
      links.setMethodCallHandler((call) async {
        if (call.method == 'link') _arrived(call.arguments as String?);
        if (call.method == 'yubikeyOtp') {
          YubikeyNfc._heard(call.arguments as String?);
        }
      });
    } else {
      // A link left from before isn't this sign-in's.
      Core().handedOverLink();
    }
    if (!await Core.openWebsite(url)) {
      _arrived(null);
      return waiting.future;
    }
    if (!Platform.isAndroid) {
      final until = DateTime.now().add(const Duration(minutes: 6));
      _polling = Timer.periodic(const Duration(milliseconds: 500), (_) {
        final link = Core().handedOverLink();
        if (link != null || DateTime.now().isAfter(until)) _arrived(link);
      });
    }
    return waiting.future;
  }

  static void _arrived(String? link) {
    _polling?.cancel();
    _polling = null;
    final waiting = _waiting;
    _waiting = null;
    if (waiting != null && !waiting.isCompleted) waiting.complete(link);
  }

  static void cancel() {
    if (Platform.isMacOS || Platform.isIOS) {
      Core.platform.invokeMethod<void>('duoCancel').catchError((_) {});
    }
    _arrived(null);
  }
}

/// A YubiKey's code over NFC, on Android (SecurityKeys.kt reads the address
/// the key gives).
class YubikeyNfc {
  static void Function(String code)? _onCode;

  static void listen(bool on, void Function(String code)? onCode) {
    if (!Platform.isAndroid) return;
    _onCode = on ? onCode : null;
    DuoBrowser.links.setMethodCallHandler((call) async {
      if (call.method == 'yubikeyOtp') _heard(call.arguments as String?);
      if (call.method == 'link') DuoBrowser._arrived(call.arguments as String?);
    });
    Core.platform
        .invokeMethod<void>('yubikeyNfc', {'on': on})
        .catchError((_) {});
  }

  static void _heard(String? code) {
    if (code != null) _onCode?.call(code);
  }
}

/// How this device reaches a security key (the core's `securityKey`), in
/// the person's words.
String _keyWords(String? here) => switch (here) {
  'windows' => 'Use your security key. Windows asks for it next.',
  'phone' =>
    'Hold your security key to the back of your phone, or plug it in, and touch it if it blinks.',
  _ => 'Plug in your security key, then touch it when it blinks.',
};

/// The words for a device that can't use a security key with PassBundle yet.
const notHere =
    'This device can\'t use a security key with PassBundle yet. Use another way to sign in, or the web vault.';

class TwoStepScreen extends StatefulWidget {
  const TwoStepScreen({
    super.key,
    required this.offer,
    required this.onSignedIn,
    required this.onStartOver,
    this.onNewDevice,
    this.calls = const TwoStepCalls(),
  });

  /// The core's answer asking for the second step: `providers`, `email`,
  /// `yubikeyNfc`, and `securityKey` (core/src/ffi.rs, `step`).
  final Map<String, dynamic> offer;
  final VoidCallback onSignedIn;
  final VoidCallback onStartOver;

  /// The new-device check holds the sign-in after its second step instead
  /// (two-step login was turned off meanwhile): its answer.
  final ValueChanged<Map<String, dynamic>>? onNewDevice;
  final TwoStepCalls calls;

  @override
  State<TwoStepScreen> createState() => _TwoStepScreenState();
}

class _TwoStepScreenState extends State<TwoStepScreen> {
  late final offered = methodsOffered(widget.offer);
  late final String? keyHere = widget.offer['securityKey'] as String?;
  late Method method = firstMethod(widget.offer);
  final code = TextEditingController();
  bool remember = false;
  bool working = false;

  /// Waiting on the security key or the browser: Cancel stops it.
  bool waiting = false;
  bool emailSent = false;
  bool choosing = false;
  String? error;

  @override
  void initState() {
    super.initState();
    _listenForYubikey();
  }

  @override
  void dispose() {
    if (method == Method.yubikey) widget.calls.yubikeyNfc(false, null);
    if (waiting) _cancel();
    code.dispose();
    super.dispose();
  }

  void _listenForYubikey() {
    widget.calls.yubikeyNfc(method == Method.yubikey, (otp) {
      if (!mounted || working || method != Method.yubikey) return;
      code.text = otp;
      _finish();
    });
  }

  void _choose(Method next) {
    if (method == Method.yubikey && next != Method.yubikey) {
      widget.calls.yubikeyNfc(false, null);
    }
    setState(() {
      method = next;
      choosing = false;
      error = null;
      code.clear();
    });
    if (next == Method.yubikey) _listenForYubikey();
  }

  /// Where a method's answer got to: signed in, or why not.
  Future<void> _after(Answer answer) async {
    if (!mounted) return;
    if (answer.ok && answer.body['step'] == 'newDevice') {
      widget.onNewDevice?.call(answer.body);
      return;
    }
    if (answer.ok) {
      if (method == Method.recoveryCode) await _recovered();
      if (mounted) widget.onSignedIn();
      return;
    }
    setState(() {
      working = false;
      waiting = false;
      // Giving up isn't an error.
      error = answer.error == 'Canceled.' ? null : answer.error;
    });
  }

  /// A recovery code turns two-step login off, as upstream's does: the
  /// person hears it before the vault opens.
  Future<void> _recovered() => showDialog<void>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text('Two-step login is off'),
      content: const Text(
        'You signed in with your recovery code, so two-step login is off for this account now. Turn it on again in the web vault, under Settings, Security, Two-step login. You\'ll get a new recovery code then.',
      ),
      actions: [
        FilledButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('OK'),
        ),
      ],
    ),
  );

  Future<void> _finish() async {
    if (working) return;
    final typed = code.text;
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(
      () => widget.calls.finish(method, typed, remember),
    );
    if (!answer.ok) code.clear();
    await _after(answer);
  }

  Future<void> _sendEmail() async {
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(widget.calls.sendEmail);
    if (!mounted) return;
    setState(() {
      working = false;
      emailSent = answer.ok;
      error = answer.ok ? null : answer.error;
    });
  }

  Future<void> _securityKey() async {
    setState(() {
      working = true;
      waiting = true;
      error = null;
    });
    final answer = await whileBusy(() => widget.calls.securityKey(remember));
    await _after(answer);
  }

  Future<void> _duo() async {
    setState(() {
      working = true;
      error = null;
    });
    final start = await whileBusy(widget.calls.duoStart);
    if (!mounted) return;
    if (!start.ok || start.body['step'] == 'done') return _after(start);
    setState(() => waiting = true);
    final link = await widget.calls.duoBrowse(start.body['url'] as String);
    if (!mounted) return;
    if (link == null) {
      setState(() {
        working = false;
        waiting = false;
        error = 'Duo wasn\'t finished. Continue with Duo to try again.';
      });
      return;
    }
    setState(() => waiting = false);
    await _after(
      await whileBusy(() => widget.calls.duoFinish(link, remember)),
    );
  }

  void _cancel() {
    if (method == Method.securityKey) widget.calls.cancelSecurityKey();
    if (method == Method.duo) widget.calls.cancelDuo();
  }

  String _subtitle() => switch (method) {
    Method.securityKey =>
      keyHere == null ? 'Use your security key.' : _keyWords(keyHere),
    Method.duo => 'Approve this sign-in with Duo, in your browser.',
    Method.yubikey =>
      isAndroid
          ? 'Touch your YubiKey, or hold it to the back of your phone.'
          : 'Click in the field, then touch your YubiKey.',
    Method.authenticator => 'Enter the code from your authenticator app.',
    Method.email =>
      emailSent
          ? 'We emailed a code to ${widget.offer['email'] ?? 'you'}.'
          : 'Get a code by email${widget.offer['email'] == null ? '' : ' at ${widget.offer['email']}'}.',
    Method.recoveryCode => 'Enter the recovery code you saved.',
  };

  Widget _errorText(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 12),
    child: Text(
      error!,
      style: TextStyle(color: Theme.of(context).colorScheme.error),
    ),
  );

  List<Widget> _field(BuildContext context) {
    final (label, numbers) = switch (method) {
      Method.yubikey => ('YubiKey code', false),
      Method.recoveryCode => ('Recovery code', false),
      _ => ('Code', true),
    };
    return [
      TextField(
        controller: code,
        enabled: !working,
        autofocus: true,
        keyboardType: numbers
            ? TextInputType.number
            : TextInputType.visiblePassword,
        autocorrect: false,
        enableSuggestions: false,
        textCapitalization: TextCapitalization.none,
        autofillHints: numbers ? const [AutofillHints.oneTimeCode] : null,
        onSubmitted: (_) => _finish(),
        decoration: InputDecoration(
          labelText: label,
          helperText: method == Method.recoveryCode
              ? 'Signing in with it turns two-step login off for this account.'
              : null,
          helperMaxLines: 3,
          errorText: error,
          // An email code that checked out can still lead to setting up
          // two-step login in the web vault, with its address.
          errorMaxLines: 6,
        ),
      ),
      const SizedBox(height: 16),
      WorkingButton(label: 'Continue', working: working, onPressed: _finish),
    ];
  }

  List<Widget> _body(BuildContext context) => switch (method) {
    Method.securityKey when keyHere == null => [
      Text(
        notHere,
        textAlign: TextAlign.center,
        style: TextStyle(color: Theme.of(context).colorScheme.error),
      ),
    ],
    Method.securityKey => [
      WorkingButton(
        label: 'Use security key',
        working: working,
        onPressed: _securityKey,
      ),
      if (error != null) _errorText(context),
    ],
    Method.duo => [
      WorkingButton(
        label: 'Continue with Duo',
        working: working,
        onPressed: _duo,
      ),
      if (waiting) ...[
        const SizedBox(height: 12),
        Text(
          'Finish in your browser. PassBundle goes on when Duo is done.',
          textAlign: TextAlign.center,
          style: TextStyle(color: context.colors.muted),
        ),
      ],
      if (error != null) _errorText(context),
    ],
    Method.email when !emailSent => [
      WorkingButton(
        label: 'Email me a code',
        working: working,
        onPressed: _sendEmail,
      ),
      if (error != null) _errorText(context),
    ],
    _ => _field(context),
  };

  @override
  Widget build(BuildContext context) {
    final others = [
      for (final m in [...offered, Method.recoveryCode])
        if (m != method) m,
    ];
    final canRemember =
        method != Method.recoveryCode &&
        (method != Method.securityKey || keyHere != null);
    return Centered(
      children: [
        heading(context, 'Two-step login', _subtitle()),
        ..._body(context),
        if (canRemember) ...[
          const SizedBox(height: 8),
          CheckboxListTile(
            value: remember,
            onChanged: working
                ? null
                : (v) => setState(() => remember = v ?? false),
            title: const Text('Don\'t ask again on this device for 30 days'),
            controlAffinity: ListTileControlAffinity.leading,
            contentPadding: EdgeInsets.zero,
          ),
        ],
        if (waiting) ...[
          const SizedBox(height: 4),
          TextButton(onPressed: _cancel, child: const Text('Cancel')),
        ],
        const SizedBox(height: 8),
        TextButton(
          onPressed: working ? null : () => setState(() => choosing = !choosing),
          child: const Text('Use another way'),
        ),
        if (choosing && !working)
          for (final m in others)
            OutlinedButton(
              key: ValueKey('two-step-${m.name}'),
              onPressed: () => _choose(m),
              child: Text(
                m == Method.securityKey && keyHere == null
                    ? '${m.label} (not on this device yet)'
                    : m.label,
              ),
            ),
        const SizedBox(height: 8),
        TextButton(
          onPressed: working ? null : widget.onStartOver,
          child: const Text('Start over'),
        ),
      ],
    );
  }
}

/// The method [TwoStepScreen] shows first: the strongest offered that this
/// device can do; a security key this device can't use only when nothing
/// else is offered, to say so.
Method firstMethod(Map<String, dynamic> offer) {
  final keyHere = offer['securityKey'] as String?;
  final offered = methodsOffered(offer);
  for (final m in offered) {
    if (m != Method.securityKey || keyHere != null) return m;
  }
  return offered.isEmpty ? Method.recoveryCode : offered.first;
}
