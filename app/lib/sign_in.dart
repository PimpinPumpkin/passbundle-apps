// Signing in (with two-step login), making an account, and unlocking the
// vault kept on this device. The master password goes to the core and the
// field is cleared at once (docs/core.md, "Memory"). People sign in to
// PassBundle's own server without being asked; a server an organization set
// by policy is used and shown; and someone hosting their own types it in,
// behind "Self-hosted?". Their server is checked as they leave its field, and
// before anything is sent to it (docs/core.md, "Checking a server"): what's
// wrong with it is said under that field, not under the master password.
import 'package:flutter/material.dart';

import 'accounts.dart';
import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

final _measuring = testSetting('PB_MEASURE') == '1';

/// A server address as someone typed it, with https:// added when they left
/// the scheme off ("vault.example.com"). An address that names http:// stays
/// as typed, so the core can say why it won't use it.
String serverAddress(String typed) {
  final address = typed.trim();
  if (address.isEmpty) return address;
  final scheme = RegExp(r'^([A-Za-z][A-Za-z0-9+.-]*)://').firstMatch(address);
  if (scheme == null) return 'https://$address';
  return scheme.group(1)!.toLowerCase() +
      address.substring(scheme.group(1)!.length);
}

class SignInScreen extends StatefulWidget {
  const SignInScreen({
    super.key,
    this.server,
    this.email,
    required this.onSignedIn,
    this.onCancel,
    this.adding = false,
  });

  final String? server;
  final String? email;
  final VoidCallback onSignedIn;

  /// Back to the vault kept on this device, when there is one.
  final VoidCallback? onCancel;

  /// Adding another account beside the ones signed in (accounts.dart).
  final bool adding;

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

  String get signInTo =>
      managed ?? (selfHosted ? serverAddress(server.text) : ours);

  /// What's wrong with the server typed in, under its field: it can't be
  /// reached, or it isn't a PassBundle server.
  String? serverError;

  /// The server field: leaving it checks the server.
  late final FocusNode serverFocus = FocusNode()
    ..addListener(() {
      if (!serverFocus.hasFocus && mounted) checkServer();
    });

  /// Counts the checks, so only the newest one's answer is shown.
  int checks = 0;

  /// Checks the server typed in, as the person leaves its field. Nothing is
  /// said about an empty field until they sign in.
  Future<void> checkServer() async {
    if (!selfHosted || managed != null || server.text.trim().isEmpty) return;
    final asked = ++checks;
    final typed = server.text;
    final answer = await Core.checkServer(serverAddress(typed));
    if (!mounted || asked != checks || typed != server.text) return;
    setState(() => serverError = answer.ok ? null : answer.error);
  }

  /// An answer about the server rather than the email or master password,
  /// when the person typed the server: said under its field. True if it was.
  bool aboutTheServer(Answer answer) {
    if (answer.ok || !answer.aboutTheServer || !selfHosted) return false;
    setState(() {
      working = false;
      error = null;
      serverError = answer.error;
    });
    return true;
  }

  @override
  void dispose() {
    serverFocus.dispose();
    super.dispose();
  }
  late final email = TextEditingController(text: widget.email ?? '');
  final password = TextEditingController();
  final code = TextEditingController();
  String? error;
  bool working = false;
  bool hidden = true;
  bool remember = false;

  /// Making an account instead (docs/core.md, "Making an account").
  bool creating = false;
  final name = TextEditingController();
  final confirm = TextEditingController();
  final hint = TextEditingController();

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
      serverError = null;
    });
    final answer = await whileBusy(
      () => Core.signIn(signInTo, email.text.trim(), typed),
    );
    if (!mounted || aboutTheServer(answer)) return;
    signedIn(answer);
  }

  /// Where signing in got to: the vault, two-step login, or why not.
  void signedIn(Answer answer) {
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

  /// Makes the account, then signs in to it, as signing in does. The
  /// passwords are checked here first, then go to the core, and the fields
  /// don't keep them.
  Future<void> createAccount() async {
    if (working) return;
    final typed = password.text;
    final again = confirm.text;
    String? wrong;
    if (email.text.trim().isEmpty) {
      wrong = 'Enter your email address.';
    } else if (typed.characters.length < 12) {
      wrong = 'Use at least 12 characters.';
    } else if (typed != again) {
      wrong = 'The two master passwords don\'t match.';
    } else if (hint.text.trim().isNotEmpty &&
        hint.text.trim().toLowerCase().contains(typed.trim().toLowerCase())) {
      wrong = 'The hint can\'t give away your master password.';
    }
    if (wrong != null) {
      setState(() => error = wrong);
      return;
    }
    password.clear();
    confirm.clear();
    setState(() {
      working = true;
      error = null;
      serverError = null;
    });
    final answer = await whileBusy(
      () => Core.signUp(
        signInTo,
        email.text.trim(),
        name.text.trim(),
        typed,
        hint.text.trim(),
      ),
    );
    if (!mounted || aboutTheServer(answer)) return;
    if (answer.body['accountMade'] == true) {
      // The account is there: signing in is what's left, with the same email.
      setState(() {
        creating = false;
        working = false;
        error = answer.error;
      });
      return;
    }
    signedIn(answer);
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

  /// Android's Back goes back a step: from two-step login or making an
  /// account to signing in, and from adding an account to the one shown,
  /// before it leaves the app. Not while a call is out.
  @override
  Widget build(BuildContext context) => PopScope(
    canPop: providers == null && !creating && widget.onCancel == null,
    onPopInvokedWithResult: (popped, _) {
      if (popped || working) return;
      if (providers != null) {
        setState(() {
          providers = null;
          error = null;
          emailSent = false;
          code.clear();
        });
      } else if (creating) {
        setState(() {
          creating = false;
          error = null;
          password.clear();
          confirm.clear();
        });
      } else {
        widget.onCancel?.call();
      }
    },
    child: _screen(context),
  );

  Widget _screen(BuildContext context) {
    final step = providers;
    if (step != null) return _secondStep(context, step);
    if (creating) return _createAccount(context);
    return Centered(
      children: [
        heading(
          context,
          widget.adding ? 'Add an account' : 'Sign in to PassBundle',
          managed == null
              ? null
              : 'Your organization\'s server: ${Uri.tryParse(managed!)?.host ?? managed}',
        ),
        if (selfHosted) _serverField(),
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
          enableSuggestions: false,
          autocorrect: false,
          enableIMEPersonalizedLearning: false,
          onSubmitted: (_) => signIn(),
          decoration: InputDecoration(
            labelText: 'Master password',
            errorText: error,
            // Room for the longest: a role that needs two-step login set
            // up in the web vault first, with its address.
            errorMaxLines: 6,
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
        const SizedBox(height: 8),
        TextButton(
          onPressed: working
              ? null
              : () => setState(() {
                  creating = true;
                  error = null;
                  password.clear();
                }),
          child: const Text('Create account'),
        ),
        if (managed == null) ...[
          const SizedBox(height: 8),
          TextButton(
            onPressed: working
                ? null
                : () => setState(() {
                    selfHosted = !selfHosted;
                    serverError = null;
                  }),
            child: Text(
              selfHosted ? 'Use PassBundle\'s server' : 'Self-hosted?',
            ),
          ),
        ],
        if (widget.onCancel != null) ...[
          const SizedBox(height: 8),
          TextButton(
            onPressed: working ? null : widget.onCancel,
            child: Text(widget.adding ? 'Cancel' : 'Back to my vault'),
          ),
        ],
      ],
    );
  }

  /// The server someone hosts themselves, with what's wrong with it.
  Widget _serverField() => Column(
    children: [
      TextField(
        key: const ValueKey('server'),
        controller: server,
        focusNode: serverFocus,
        enabled: !working,
        keyboardType: TextInputType.url,
        autocorrect: false,
        onChanged: (_) {
          if (serverError != null) setState(() => serverError = null);
        },
        decoration: InputDecoration(
          labelText: 'Server',
          hintText: 'vault.example.com',
          errorText: serverError,
          errorMaxLines: 4,
        ),
      ),
      const SizedBox(height: 12),
    ],
  );

  Widget _hideButton() => IconButton(
    tooltip: hidden ? 'Show' : 'Hide',
    icon: Icon(
      hidden ? Icons.visibility_outlined : Icons.visibility_off_outlined,
    ),
    onPressed: () => setState(() => hidden = !hidden),
  );

  /// Making an account: the email (any address the server takes), a name,
  /// the master password twice, and a hint, on the same server choice as
  /// signing in.
  Widget _createAccount(BuildContext context) => Centered(
    children: [
      heading(
        context,
        'Create a PassBundle account',
        managed == null
            ? null
            : 'Your organization\'s server: ${Uri.tryParse(managed!)?.host ?? managed}',
      ),
      if (selfHosted) _serverField(),
      TextField(
        controller: email,
        enabled: !working,
        keyboardType: TextInputType.emailAddress,
        autofillHints: const [AutofillHints.email],
        decoration: const InputDecoration(labelText: 'Email'),
      ),
      const SizedBox(height: 12),
      TextField(
        controller: name,
        enabled: !working,
        textCapitalization: TextCapitalization.words,
        autofillHints: const [AutofillHints.name],
        decoration: const InputDecoration(labelText: 'Name (optional)'),
      ),
      const SizedBox(height: 12),
      TextField(
        controller: password,
        obscureText: hidden,
        enabled: !working,
        enableSuggestions: false,
        autocorrect: false,
        enableIMEPersonalizedLearning: false,
        decoration: InputDecoration(
          labelText: 'Master password',
          helperText: 'At least 12 characters',
          suffixIcon: _hideButton(),
        ),
      ),
      const SizedBox(height: 12),
      TextField(
        controller: confirm,
        obscureText: hidden,
        enabled: !working,
        enableSuggestions: false,
        autocorrect: false,
        enableIMEPersonalizedLearning: false,
        decoration: const InputDecoration(labelText: 'Master password again'),
      ),
      const SizedBox(height: 12),
      TextField(
        controller: hint,
        enabled: !working,
        onSubmitted: (_) => createAccount(),
        decoration: InputDecoration(
          labelText: 'Hint (optional)',
          helperText: 'Something that reminds you, never the password',
          errorText: error,
          errorMaxLines: 3,
        ),
      ),
      const SizedBox(height: 12),
      Text(
        'Your master password opens your vault, and only you know it: PassBundle can\'t see it or reset it if you forget it.',
        style: TextStyle(color: context.colors.muted, fontSize: 13),
      ),
      const SizedBox(height: 16),
      WorkingButton(
        label: 'Create account',
        working: working,
        onPressed: createAccount,
      ),
      const SizedBox(height: 8),
      TextButton(
        onPressed: working
            ? null
            : () => setState(() {
                creating = false;
                error = null;
                password.clear();
                confirm.clear();
              }),
        child: const Text('I have an account: sign in'),
      ),
      if (managed == null)
        TextButton(
          onPressed: working
              ? null
              : () => setState(() {
                    selfHosted = !selfHosted;
                    serverError = null;
                  }),
          child: Text(selfHosted ? 'Use PassBundle\'s server' : 'Self-hosted?'),
        ),
    ],
  );

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
            showSelectedIcon: false,
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
              // An email code that checked out can still lead to setting up
              // two-step login in the web vault, with its address.
              errorMaxLines: 6,
            ),
          ),
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
    this.menu,
    this.promptBiometric = false,
    this.onPrompted,
  });

  final String email;
  final String server;

  /// On a phone, as the app opens or comes back to the front locked: ask
  /// for the biometric by itself, once ([onPrompted] says it was), rather
  /// than bringing up the keyboard. Not after the person pressed Lock.
  final bool promptBiometric;
  final VoidCallback? onPrompted;

  /// The accounts signed in here: with more than one, switching to another
  /// (it stays as it is; this one stays locked).
  final AccountsMenu? menu;

  /// How long the master password took to unlock, in milliseconds, or null
  /// when biometrics unlocked it.
  final ValueChanged<int?> onUnlocked;
  final VoidCallback onSignOut;

  @override
  State<UnlockScreen> createState() => _UnlockScreenState();
}

class _UnlockScreenState extends State<UnlockScreen> {
  final password = TextEditingController(
    text: _measuring ? (testSetting('PB_MEASURE_PASSWORD') ?? '') : '',
  );
  final passwordFocus = FocusNode();
  String? error;
  bool working = false;
  bool hidden = true;

  /// "Windows Hello" or "Touch ID" when this account can unlock with it here.
  String? method;

  /// The prompt asked for by itself is up ([promptIfAsked]).
  bool prompting = false;

  @override
  void initState() {
    super.initState();
    if (_measuring) {
      WidgetsBinding.instance.addPostFrameCallback((_) => unlock());
    } else {
      Core.biometricStatus().then((status) {
        if (!mounted) return;
        final setUp = status['status'] == 'setUp';
        if (setUp) setState(() => method = status['method'] as String);
        // On a phone the field waited for this, so the keyboard doesn't come
        // up under the biometric prompt.
        if (setUp) {
          promptIfAsked();
        } else if (isPhone) {
          passwordFocus.requestFocus();
        }
      });
    }
  }

  @override
  void didUpdateWidget(UnlockScreen old) {
    super.didUpdateWidget(old);
    // Back to the front, locked, with this screen already showing: once
    // it's drawn again.
    if (widget.promptBiometric && !old.promptBiometric) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) promptIfAsked();
      });
    }
  }

  @override
  void dispose() {
    passwordFocus.dispose();
    super.dispose();
  }

  /// The phone's own prompt, by itself, when the app asks for it (it opened
  /// or came back locked): once. Cancelling it leaves the master password.
  void promptIfAsked() {
    if (!isPhone ||
        !widget.promptBiometric ||
        method == null ||
        working ||
        prompting) {
      return;
    }
    prompting = true;
    widget.onPrompted?.call();
    unlockWithBiometric(byItself: true).whenComplete(() => prompting = false);
  }

  /// The master password hint, from the account's server: emailed to the
  /// account's email, or shown here when the server is set to show it.
  Future<void> getHint() async {
    setState(() => working = true);
    final answer = await whileBusy(Core.passwordHint);
    if (!mounted) return;
    setState(() => working = false);
    final hint = answer.body['hint'] as String?;
    await showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Master password hint'),
        content: Text(
          !answer.ok
              ? answer.error ?? 'Couldn\'t get your hint. Try again.'
              : hint != null
              ? 'Your hint: $hint'
              : 'If your account has a hint, your server emailed it to ${widget.email}.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('OK'),
          ),
        ],
      ),
    );
  }

  /// Unlocks with the phone's biometrics, or the desktop agent's. Asked
  /// for [byItself] (the app opened or came back locked), the field stays
  /// as it is meanwhile, and a cancel brings up the keyboard for the master
  /// password instead.
  Future<void> unlockWithBiometric({bool byItself = false}) async {
    setState(() {
      if (!byItself) working = true;
      error = null;
    });
    final answer = await whileBusy(Core.unlockWithBiometric);
    if (!mounted) return;
    if (answer.ok && answer.body['unlocked'] == true) {
      widget.onUnlocked(null);
      return;
    }
    setState(() {
      working = false;
      error = answer.ok ? null : answer.error;
      // An out-of-date setup was turned off: the master password, then turn it on again.
      if (!answer.ok) method = null;
    });
    if (byItself) passwordFocus.requestFocus();
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
        focusNode: passwordFocus,
        obscureText: hidden,
        // On a phone, once it's known there's no biometric prompt to come.
        autofocus: !isPhone,
        enabled: !working,
        enableSuggestions: false,
        autocorrect: false,
        enableIMEPersonalizedLearning: false,
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
      // First run, or a while away: the hint the account was made with.
      TextButton(
        key: const ValueKey('password-hint'),
        onPressed: working ? null : getHint,
        child: const Text('Get your master password hint'),
      ),
      const SizedBox(height: 6),
      Text(
        Uri.tryParse(widget.server)?.host ?? widget.server,
        textAlign: TextAlign.center,
        style: TextStyle(color: context.colors.muted, fontSize: 12),
      ),
      if (widget.menu case final menu? when menu.several)
        Center(
          child: AccountMenuButton(
            menu: menu,
            // The lock screen's own Sign out is below.
            signOut: false,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
              child: Text(
                'Switch account',
                style: TextStyle(
                  color: Theme.of(context).colorScheme.primary,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
          ),
        ),
      TextButton(
        onPressed: working
            ? null
            : () async {
                final sure = await askToSignOut(
                  context,
                  method: method,
                  email: widget.email,
                  others: widget.menu?.several ?? false,
                );
                if (sure) widget.onSignOut();
              },
        child: const Text('Sign out'),
      ),
    ],
  );
}
