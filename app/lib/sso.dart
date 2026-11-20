// Single sign-on from the sign-in screen (DESIGN.md in the meta repo, "Sign
// in with Microsoft Entra ID", "In the desktop and phone apps"; the core's
// core/src/sso.rs). The email finds the organization, or its SSO identifier
// is typed; the person signs in in their own browser, never a web view (a
// computer's default browser, whose answer comes back to the core's
// listener on this computer; an iPhone's sign-in sheet; an Android Custom
// Tab, whose answer comes back as a link); then the master password
// unlocks, as after any single sign-on. Two-step login, when the account
// has it, comes between, on the two-step screen (two_step.dart).
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'two_step.dart';
import 'widgets.dart';

/// What the phones' answer starts with: the redirect the server takes for
/// them, a private-use scheme named after the app (core/src/sso.rs).
const ssoLink = 'com.passbundle.app:/sso?';

/// A password sign-in that an organization requires single sign-on for.
const ssoRequired =
    'Your organization requires you to sign in with single sign-on.';

/// While the browser is out.
const ssoInBrowser =
    'Finish in your browser. PassBundle goes on when you\'re signed in.';

const _noBrowser =
    'PassBundle couldn\'t open your browser. Check that this device has a default browser, then try again.';

/// What the core and the browser do for single sign-on. Tests stand in for
/// the browser.
class SsoCalls {
  const SsoCalls();

  Future<Answer> identifierFor(String server, String email) =>
      Core.ssoIdentifier(server, email);

  Future<Answer> start(String server, String email, String identifier) =>
      Core.ssoStart(server, email, identifier);

  /// The browser, with the core's [start] answer (`url`, `listens`), then
  /// the core with its answer: as [Core.signIn] answers. On a computer, the
  /// default browser, whose answer the core takes at its listener; on a
  /// phone, the system's sign-in sheet or Custom Tab ([SsoBrowser]), whose
  /// link goes to the core.
  Future<Answer> browse(Map<String, dynamic> start) async {
    final url = start['url'] as String;
    if (start['listens'] == true) {
      if (!await Core.openWebsite(url)) {
        // The core's listener goes, and the sign-in with it.
        Core().ssoCancel();
        await Core.ssoWait();
        return Answer({'ok': false, 'error': _noBrowser});
      }
      return Core.ssoWait();
    }
    final link = await SsoBrowser.open(url);
    return Core.ssoFinish(link ?? '');
  }

  /// Stops the browser's wait: it answers that single sign-on wasn't finished.
  void cancel() {
    Core().ssoCancel();
    SsoBrowser.cancel();
  }

  Future<Answer> unlock(String password) => Core.ssoUnlock(password);
}

/// The phones' browser for single sign-on: on an iPhone, the system's
/// sign-in sheet (ASWebAuthenticationSession, AppDelegate.swift), which
/// hands back the link itself; on Android, a Custom Tab
/// (MainActivity.kt), whose link opens the app again (LinkActivity.kt).
/// Back in the app without a link is a canceled sign-in.
class SsoBrowser {
  static Completer<String?>? _waiting;
  static AppLifecycleListener? _returns;
  static Timer? _grace;

  /// The link, or null if the person closed the browser without one.
  static Future<String?> open(String url) async {
    cancel();
    if (Platform.isIOS) {
      return Core.platform.invokeMethod<String>('ssoSignIn', {'url': url});
    }
    final waiting = _waiting = Completer<String?>();
    DuoBrowser.links.setMethodCallHandler((call) async {
      if (call.method != 'link') return;
      final link = call.arguments as String?;
      if (link != null && link.startsWith(ssoLink)) _arrived(link);
    });
    final opened =
        await Core.platform.invokeMethod<bool>('openAuthTab', {'url': url}) ??
        false;
    if (!opened) {
      _arrived(null);
      return waiting.future;
    }
    // Back in the app: the link, when there's one, comes first; without one
    // by then, the person closed the browser.
    _returns = AppLifecycleListener(
      onResume: () {
        _grace?.cancel();
        _grace = Timer(const Duration(seconds: 2), () => _arrived(null));
      },
    );
    return waiting.future;
  }

  static void _arrived(String? link) {
    _grace?.cancel();
    _grace = null;
    _returns?.dispose();
    _returns = null;
    final waiting = _waiting;
    _waiting = null;
    if (waiting != null && !waiting.isCompleted) waiting.complete(link);
  }

  static void cancel() {
    if (Platform.isIOS) {
      Core.platform.invokeMethod<void>('ssoCancel').catchError((_) {});
    }
    _arrived(null);
  }
}

/// Single sign-on's first step: the organization's SSO identifier (found by
/// the email, typed, or given by a password sign-in's answer when an
/// organization requires it), then the browser.
class SsoScreen extends StatefulWidget {
  const SsoScreen({
    super.key,
    required this.server,
    required this.email,
    this.identifier,
    this.mustUse = false,
    required this.onAnswer,
    required this.onBack,
    this.calls = const SsoCalls(),
  });

  final String server;
  final String email;

  /// The SSO identifier, when it's known already: filled in, and the screen
  /// waits for Continue. Without one, the email's organization is looked for
  /// first, and its sign-in starts when there's one.
  final String? identifier;

  /// A password sign-in was answered that the organization requires single
  /// sign-on: the screen says so.
  final bool mustUse;

  /// Where single sign-on got to: the master password next, two-step login,
  /// as [Core.signIn] answers.
  final ValueChanged<Answer> onAnswer;
  final VoidCallback onBack;
  final SsoCalls calls;

  @override
  State<SsoScreen> createState() => _SsoScreenState();
}

class _SsoScreenState extends State<SsoScreen> {
  late final identifier = TextEditingController(text: widget.identifier ?? '');

  /// Asking the server which organization the email is at.
  late bool finding = widget.identifier == null;
  bool working = false;

  /// The browser is out: Cancel stops its wait.
  bool browsing = false;
  String? error;

  @override
  void initState() {
    super.initState();
    if (finding) _find();
  }

  @override
  void dispose() {
    if (browsing) widget.calls.cancel();
    identifier.dispose();
    super.dispose();
  }

  /// The organization whose verified domain the email is at: its sign-in
  /// starts. Without one, the identifier is typed.
  Future<void> _find() async {
    final answer = await whileBusy(
      () => widget.calls.identifierFor(widget.server, widget.email),
    );
    if (!mounted) return;
    final found = answer.ok ? answer.body['identifier'] as String? : null;
    setState(() {
      finding = false;
      if (found != null) identifier.text = found;
      // It couldn't ask (offline, say): said, and the identifier can still
      // be typed.
      if (!answer.ok) error = answer.error;
    });
    if (found != null) await _continue();
  }

  Future<void> _continue() async {
    if (working || finding) return;
    setState(() {
      working = true;
      error = null;
    });
    final start = await whileBusy(
      () => widget.calls.start(widget.server, widget.email, identifier.text),
    );
    if (!mounted) return;
    if (!start.ok) {
      setState(() {
        working = false;
        error = start.error;
      });
      return;
    }
    setState(() => browsing = true);
    // Not busy: the browser can take minutes, and the core's other calls
    // don't wait on it.
    final answer = await widget.calls.browse(start.body);
    if (!mounted) return;
    setState(() {
      working = false;
      browsing = false;
      error = answer.ok ? null : answer.error;
    });
    if (answer.ok) widget.onAnswer(answer);
  }

  @override
  Widget build(BuildContext context) => Centered(
    children: [
      heading(
        context,
        'Single sign-on',
        widget.email,
        detail: Uri.tryParse(widget.server)?.host ?? widget.server,
      ),
      if (widget.mustUse) ...[
        const Text(ssoRequired, textAlign: TextAlign.center),
        const SizedBox(height: 16),
      ],
      TextField(
        key: const ValueKey('sso-identifier'),
        controller: identifier,
        enabled: !working && !finding,
        autocorrect: false,
        enableSuggestions: false,
        keyboardType: TextInputType.visiblePassword,
        onSubmitted: (_) => _continue(),
        decoration: InputDecoration(
          labelText: 'SSO identifier',
          helperText: 'Your organization\'s, from your administrator',
          errorText: error,
          errorMaxLines: 6,
        ),
      ),
      const SizedBox(height: 16),
      WorkingButton(
        label: 'Continue',
        working: working || finding,
        onPressed: _continue,
      ),
      if (browsing) ...[
        const SizedBox(height: 12),
        Text(
          ssoInBrowser,
          textAlign: TextAlign.center,
          style: TextStyle(color: context.colors.muted),
        ),
        TextButton(
          onPressed: widget.calls.cancel,
          child: const Text('Cancel'),
        ),
      ],
      const SizedBox(height: 8),
      TextButton(
        onPressed: working ? null : widget.onBack,
        child: Text(
          widget.mustUse ? 'Back' : 'Sign in with your master password',
        ),
      ),
    ],
  );
}

/// Single sign-on signed the person in: their master password unlocks, on
/// this device (the core opens the account key with it). A wrong one says
/// so, and can be tried again.
class SsoUnlockScreen extends StatefulWidget {
  const SsoUnlockScreen({
    super.key,
    required this.email,
    required this.server,
    required this.onSignedIn,
    required this.onStartOver,
    this.calls = const SsoCalls(),
  });

  /// The account's email, as its server keeps it.
  final String email;
  final String server;
  final VoidCallback onSignedIn;
  final VoidCallback onStartOver;
  final SsoCalls calls;

  @override
  State<SsoUnlockScreen> createState() => _SsoUnlockScreenState();
}

class _SsoUnlockScreenState extends State<SsoUnlockScreen> {
  final password = TextEditingController();
  bool working = false;
  bool hidden = true;
  String? error;

  @override
  void dispose() {
    password.clear();
    password.dispose();
    super.dispose();
  }

  Future<void> _unlock() async {
    if (working) return;
    final typed = password.text;
    // The core has it now; the field doesn't keep it.
    password.clear();
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(() => widget.calls.unlock(typed));
    if (!mounted) return;
    if (answer.ok) {
      widget.onSignedIn();
      return;
    }
    setState(() {
      working = false;
      error = answer.error;
    });
  }

  @override
  Widget build(BuildContext context) => Centered(
    children: [
      heading(
        context,
        'Unlock with your master password',
        widget.email,
        detail: Uri.tryParse(widget.server)?.host ?? widget.server,
      ),
      TextField(
        key: const ValueKey('sso-master-password'),
        controller: password,
        obscureText: hidden,
        autofocus: !isPhone,
        enabled: !working,
        enableSuggestions: false,
        autocorrect: false,
        enableIMEPersonalizedLearning: false,
        onSubmitted: (_) => _unlock(),
        decoration: InputDecoration(
          labelText: 'Master password',
          helperText:
              'Single sign-on signed you in. Your master password unlocks your vault.',
          helperMaxLines: 3,
          errorText: error,
          errorMaxLines: 4,
          suffixIcon: IconButton(
            tooltip: hidden ? 'Show' : 'Hide',
            icon: Icon(
              hidden ? Icons.visibility_outlined : Icons.visibility_off_outlined,
            ),
            onPressed: () => setState(() => hidden = !hidden),
          ),
        ),
      ),
      const SizedBox(height: 16),
      WorkingButton(label: 'Unlock', working: working, onPressed: _unlock),
      const SizedBox(height: 8),
      TextButton(
        onPressed: working ? null : widget.onStartOver,
        child: const Text('Start over'),
      ),
    ],
  );
}
