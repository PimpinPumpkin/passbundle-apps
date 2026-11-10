// Signing in (with two-step login, and the new-device check on a device the
// account has never used), making an account, and unlocking the vault kept
// on this device. The master password goes to the core and the
// field is cleared at once (docs/core.md, "Memory"). People sign in to
// PassBundle's own server without being asked; a server an organization set
// by policy is used and shown; and someone hosting their own types it in,
// behind "Self-hosted?". Their server is checked as they leave its field, and
// before anything is sent to it (docs/core.md, "Checking a server"): what's
// wrong with it is said under that field, not under the master password.
// A server that says it takes no new accounts isn't offered the form for
// making one: the screen says so where "Create account" would be, in the web
// vault's words, and on PassBundle's server links to the website's contact
// page.
import 'package:flutter/material.dart';

import 'accounts.dart';
import 'core.dart';
import 'new_device.dart';
import 'theme.dart';
import 'two_step.dart';
import 'widgets.dart';

final _measuring = testSetting('PB_MEASURE') == '1';

// What's said where "Create account" would be on a server that takes no new
// accounts, and under an email the server turned down: the web vault's
// sentences, word for word (passbundle-clients, brand/messages.en.json, the
// keys starting keykeeperNewAccounts).
const _notOpenYet = 'New accounts aren\'t open yet.';
const _askForOne = 'Ask for one';
const _ownServer =
    'This server isn\'t taking new accounts. Ask the person who runs it.';
const _notThatAddress = 'New accounts aren\'t open for that address.';
const _notThatAddressOwnServer =
    'New accounts aren\'t open for that address. Ask the person who runs this server.';

/// Tests only: where to sign in, in place of the core's answer
/// ([Core.servers]), so a stand-in on this computer can be PassBundle's own
/// server. Null is what the core says.
@visibleForTesting
Map<String, dynamic>? debugServers;

/// Under flutter test, the screen asks a server whether it takes new accounts
/// as it opens only where the test names the servers ([debugServers]): no
/// test asks PassBundle's. A product build never takes FLUTTER_TEST from its
/// environment.
final _underTest = testSetting('FLUTTER_TEST') != null;

/// A server's address as it's compared with another: the host in lower case,
/// without a slash at the end.
String _sameServer(String address) {
  final typed = address.trim();
  return (Uri.tryParse(typed)?.toString() ?? typed).replaceAll(
    RegExp(r'/+$'),
    '',
  );
}

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
  final servers = debugServers ?? Core().servers();
  late final ours = servers['ours'] as String;
  late final managed = servers['managed'] as String?;

  /// The website's page for asking for an account. Only an https address is
  /// opened, as in the web vault; without one there's no link.
  late final String? contactPage = switch (servers['contactPage']) {
    final String page when page.startsWith('https://') => page,
    _ => null,
  };

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
    setState(() {
      serverError = answer.ok ? null : answer.error;
      heard(serverAddress(typed), answer);
    });
  }

  /// The servers that said they take no new accounts, with their answer to
  /// the check (docs/core.md, "Checking a server"). One that doesn't say, or
  /// can't be asked, isn't here: the form is offered, and the server answers
  /// it.
  final closed = <String>{};

  /// The server signed in to takes no new accounts.
  bool get closedHere => closed.contains(_sameServer(signInTo));

  /// PassBundle's own server, not one someone else runs.
  bool get onOurs => _sameServer(signInTo) == _sameServer(ours);

  /// Why the server turned down the email at the form's first step, when it
  /// said: "closed" or "address". Said under the email until it changes.
  String? refused;

  /// The browser didn't open the contact page.
  bool couldntOpen = false;

  /// Takes what a server said about new accounts with its answer to the
  /// check. Inside setState.
  void heard(String server, Answer answer) {
    if (!answer.ok) return;
    if (answer.body['newAccounts'] == 'closed') {
      closed.add(_sameServer(server));
    } else {
      closed.remove(_sameServer(server));
    }
    leaveFormIfClosed();
  }

  /// Asks the server signed in to whether it takes new accounts: as the
  /// screen opens, and when the person turns to the other server. Nothing is
  /// said if it can't be asked.
  Future<void> askAboutNewAccounts() async {
    if (_underTest && debugServers == null) return;
    final server = signInTo;
    if (server.isEmpty) return;
    final answer = await Core.checkServer(server);
    if (mounted) setState(() => heard(server, answer));
  }

  /// The form for making an account goes once its server is known to take
  /// none: back to signing in, which says so. Inside setState.
  void leaveFormIfClosed() {
    if (!creating || !closedHere) return;
    creating = false;
    error = null;
    refused = null;
    password.clear();
    confirm.clear();
  }

  /// PassBundle's server or the person's own: the other one.
  void switchServer() {
    setState(() {
      selfHosted = !selfHosted;
      serverError = null;
      refused = null;
      leaveFormIfClosed();
    });
    askAboutNewAccounts();
  }

  /// Opens the website's contact page in the browser.
  Future<void> askForOne(String page) async {
    final opened = await Core.openWebsite(page);
    if (mounted) setState(() => couldntOpen = !opened);
  }

  @override
  void initState() {
    super.initState();
    askAboutNewAccounts();
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
  String? error;
  bool working = false;
  bool hidden = true;

  /// Making an account instead (docs/core.md, "Making an account").
  bool creating = false;
  final name = TextEditingController();
  final confirm = TextEditingController();
  final hint = TextEditingController();

  /// The second step, once the server asks for one: what it offers (two_step.dart).
  Map<String, dynamic>? twoStep;

  /// This device is new to the account: the check holding the sign-in
  /// (new_device.dart).
  Map<String, dynamic>? newDevice;

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

  /// Where signing in got to: the vault, two-step login, the new-device
  /// check, or why not.
  void signedIn(Answer answer) {
    if (answer.ok && answer.body['step'] == 'twoFactor') {
      setState(() {
        working = false;
        twoStep = answer.body;
        newDevice = null;
        error = null;
      });
    } else if (answer.ok && answer.body['step'] == 'newDevice') {
      setState(() {
        working = false;
        newDevice = answer.body;
        twoStep = null;
        error = null;
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
      refused = null;
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
    final why = answer.body['newAccounts'];
    if (!answer.ok && (why == 'closed' || why == 'address')) {
      // The server turned the email down, and said why: the web vault's
      // sentence for it, under the email.
      setState(() {
        working = false;
        error = null;
        refused = why as String;
      });
      return;
    }
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

  /// Android's Back goes back a step: from two-step login, the new-device
  /// check, or making an account to signing in, and from adding an account
  /// to the one shown, before it leaves the app. Not while a call is out.
  @override
  Widget build(BuildContext context) => PopScope(
    canPop:
        twoStep == null &&
        newDevice == null &&
        !creating &&
        widget.onCancel == null,
    onPopInvokedWithResult: (popped, _) {
      if (popped || working) return;
      if (newDevice != null) {
        setState(() {
          newDevice = null;
          error = null;
        });
      } else if (twoStep != null) {
        // The two-step screen stops a security key or Duo it was waiting on.
        setState(() {
          twoStep = null;
          error = null;
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
    final held = newDevice;
    if (held != null) {
      return NewDeviceScreen(
        // Each sign-in's check starts afresh.
        key: ObjectKey(held),
        answer: held,
        onSignedIn: widget.onSignedIn,
        onTwoStep: (offer) => setState(() {
          newDevice = null;
          twoStep = offer;
        }),
        onStartOver: () => setState(() => newDevice = null),
      );
    }
    final step = twoStep;
    if (step != null) {
      return TwoStepScreen(
        // Each sign-in's second step starts afresh.
        key: ObjectKey(step),
        offer: step,
        onSignedIn: widget.onSignedIn,
        onNewDevice: (held) => setState(() {
          twoStep = null;
          newDevice = held;
        }),
        onStartOver: () => setState(() => twoStep = null),
      );
    }
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
        if (closedHere)
          // The server takes no new accounts: who to ask, not a form that
          // can only fail. With the link, as tall as "Create account", so
          // nothing moves as the answer comes.
          Padding(
            padding: EdgeInsets.symmetric(
              vertical: onOurs && contactPage != null ? 0 : 12,
            ),
            child: _noNewAccounts('closed'),
          )
        else
          TextButton(
            onPressed: working
                ? null
                : () => setState(() {
                    creating = true;
                    error = null;
                    refused = null;
                    password.clear();
                  }),
            child: const Text('Create account'),
          ),
        if (managed == null) ...[
          const SizedBox(height: 8),
          TextButton(
            onPressed: working ? null : switchServer,
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
        // Another address is another server: what was said about the last
        // one goes, and what's known about this one shows.
        onChanged: (_) => setState(() {
          serverError = null;
          refused = null;
          leaveFormIfClosed();
        }),
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
        onChanged: (_) {
          if (refused != null) setState(() => refused = null);
        },
        decoration: const InputDecoration(labelText: 'Email'),
      ),
      if (refused case final why?)
        Padding(
          padding: const EdgeInsets.only(left: 14, top: 4),
          child: _noNewAccounts(why, underEmail: true),
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
          onPressed: working ? null : switchServer,
          child: Text(selfHosted ? 'Use PassBundle\'s server' : 'Self-hosted?'),
        ),
    ],
  );

  /// A server that takes no new accounts ([why] "closed"), or none for the
  /// email just tried ("address"): the web vault's sentence for it. On
  /// PassBundle's server, with its link to the website's contact page; on
  /// anyone else's, the person who runs it is who to ask.
  Widget _noNewAccounts(String why, {bool underEmail = false}) {
    final words = switch ((onOurs, why)) {
      (true, 'address') => _notThatAddress,
      (true, _) => _notOpenYet,
      (false, 'address') => _notThatAddressOwnServer,
      (false, _) => _ownServer,
    };
    final page = onOurs ? contactPage : null;
    final theme = Theme.of(context);
    final small = theme.textTheme.bodySmall;
    return Column(
      key: const ValueKey('no-new-accounts'),
      crossAxisAlignment: underEmail
          ? CrossAxisAlignment.start
          : CrossAxisAlignment.center,
      children: [
        Wrap(
          alignment: underEmail ? WrapAlignment.start : WrapAlignment.center,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            Text(
              words,
              textAlign: underEmail ? TextAlign.start : TextAlign.center,
              style: underEmail
                  ? small?.copyWith(color: theme.colorScheme.error)
                  : null,
            ),
            if (page != null)
              // Next to the sentence, as a link in it; under the email, at
              // the sentence's size.
              TextButton(
                key: const ValueKey('ask-for-account'),
                onPressed: working ? null : () => askForOne(page),
                style: underEmail
                    ? TextButton.styleFrom(
                        padding: const EdgeInsets.symmetric(horizontal: 6),
                        minimumSize: const Size(0, 32),
                        tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                        textStyle: small?.copyWith(fontWeight: FontWeight.w600),
                      )
                    : TextButton.styleFrom(
                        padding: const EdgeInsets.symmetric(horizontal: 6),
                      ),
                child: const Text(_askForOne),
              ),
          ],
        ),
        if (page != null && couldntOpen)
          Text(
            'Couldn\'t open $page.',
            textAlign: underEmail ? TextAlign.start : TextAlign.center,
            style: TextStyle(color: context.colors.muted, fontSize: 13),
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

  /// The phone's own prompt, or the Mac's Touch ID, by itself, when the app
  /// asks for it (it opened or came back locked): once. Cancelling it leaves
  /// the master password.
  void promptIfAsked() {
    if (!ownBiometrics ||
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

  /// Unlocks with the phone's biometrics, the Mac's Touch ID, or the
  /// desktop agent's Windows Hello. Asked
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
          // Face ID (or a phone's face unlock) by its face, as the offer
          // to turn it on shows it; a fingerprint otherwise.
          icon: Icon(
            method!.toLowerCase().contains('face')
                ? Icons.face_unlock_outlined
                : Icons.fingerprint_rounded,
          ),
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
