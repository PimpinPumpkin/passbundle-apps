// Approving a sign-in from another device (docs/core.md, "Approving a
// sign-in"; DESIGN.md in the meta repo): someone signing in on the web vault
// or the browser extension picks "Log in with device", and approves it here.
// While the vault is open and the app is on screen, it asks the server every
// 10 seconds for sign-ins waiting for this account's answer, and shows each
// new one once, as a prompt over the vault; Settings lists them all. The core
// makes the fingerprint phrase, wraps the key, and refuses an approval the
// person didn't confirm; this only draws, and asks.
//
// Phones show a request while PassBundle is open, or the next time it opens:
// a prompt while it's closed needs push notifications, which wait on
// PassBundle's own push service (docs/phones.md).
import 'dart:async';

import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

/// A sign-in waiting for this account's answer, as the core listed it.
class SignInRequest {
  SignInRequest(Map<String, dynamic> json, {DateTime? listedAt})
    : id = json['id'] as String,
      device = json['device'] as String? ?? 'A device',
      ip = json['ip'] as String? ?? '',
      fingerprint = json['fingerprint'] as String,
      _age = Duration(seconds: (json['ageSeconds'] as num?)?.toInt() ?? 0),
      _left = Duration(seconds: (json['secondsLeft'] as num?)?.toInt() ?? 0),
      _listedAt = listedAt ?? signInClock();

  final String id;

  /// The requesting device, in words ("Web vault in Chrome").
  final String device;

  /// Its address, as the server saw it.
  final String ip;

  /// The phrase both devices show.
  final String fingerprint;
  final Duration _age;
  final Duration _left;
  final DateTime _listedAt;

  Duration get age => _age + signInClock().difference(_listedAt);

  Duration get left {
    final left = _left - signInClock().difference(_listedAt);
    return left.isNegative ? Duration.zero : left;
  }

  bool get expired => left == Duration.zero;
}

/// The clock requests count down by (tests set it).
DateTime Function() signInClock = DateTime.now;

/// "Just now", "3 minutes ago".
String askedAgo(Duration age) => age.inMinutes < 1
    ? 'Just now'
    : age.inMinutes == 1
    ? '1 minute ago'
    : '${age.inMinutes} minutes ago';

/// "14 minutes", "1 minute", "Under a minute".
String timeLeft(Duration left) => left.inMinutes < 1
    ? 'Under a minute'
    : left.inMinutes == 1
    ? '1 minute'
    : '${left.inMinutes} minutes';

/// What the prompt and Settings ask of the core, which tests stand in for.
abstract class SignInRequestSource {
  /// The requests waiting, from the server (`ok`, `requests`).
  Future<Answer> list();

  /// Approves or denies one; approving may answer `confirmFirst`.
  Future<Answer> answer(String id, String fingerprint, bool approve);

  /// Asks the person to confirm it's them: biometrics, else the master
  /// password. True once they have.
  Future<bool> confirm(BuildContext context);

  /// Whether this device asks them to before approving.
  bool get confirmApprovals;
  set confirmApprovals(bool on);
}

/// The core's.
class CoreSignInRequests implements SignInRequestSource {
  @override
  Future<Answer> list() => Core.signInRequests();

  @override
  Future<Answer> answer(String id, String fingerprint, bool approve) =>
      whileBusy(() => Core.answerSignInRequest(id, fingerprint, approve));

  @override
  Future<bool> confirm(BuildContext context) async {
    final status = await Core.biometricStatus();
    if (status['status'] == 'setUp') {
      final answer = await whileBusy(
        () => Core.unlockWithBiometric(reason: 'Approve sign-in'),
      );
      if (answer.ok && answer.body['unlocked'] == true) return true;
    }
    if (!context.mounted) return false;
    return await showDialog<bool>(
          context: context,
          builder: (_) => const _ConfirmWithMasterPassword(),
        ) ==
        true;
  }

  @override
  bool get confirmApprovals => Core().confirmApprovals();

  @override
  set confirmApprovals(bool on) => Core().setConfirmApprovals(on);
}

/// The requests waiting, asked for every 10 seconds while the vault is open
/// and the app is on screen ([start], [pause], [resume], [stop]). Each new
/// one is prompted for once ([unprompted]).
class SignInRequests extends ChangeNotifier {
  SignInRequests([SignInRequestSource? source])
    : source = source ?? CoreSignInRequests();

  SignInRequestSource source;
  List<SignInRequest> requests = const [];
  final _prompted = <String>{};
  Timer? _timer;
  bool _asking = false;
  bool _running = false;

  /// A check skipped beside a slow call, made as soon as it's done.
  bool _skipped = false;

  static const every = Duration(seconds: 10);

  /// The vault opened: asks now, then every 10 seconds.
  void start() {
    if (!_running) busy.addListener(_busyChanged);
    _running = true;
    resume();
  }

  void _busyChanged() {
    if (!busy.value && _skipped) {
      _skipped = false;
      check();
    }
  }

  /// The app left the screen: stops asking until it's back.
  void pause() {
    _timer?.cancel();
    _timer = null;
  }

  void resume() {
    if (!_running) return;
    _timer?.cancel();
    _timer = Timer.periodic(every, (_) => check());
    check();
  }

  /// The vault closed: forgets every request.
  void stop() {
    if (_running) busy.removeListener(_busyChanged);
    _running = false;
    _skipped = false;
    pause();
    requests = const [];
    _prompted.clear();
    notifyListeners();
  }

  Future<void> check() async {
    // Never beside a slow call (a sync, a save), nor twice at once.
    if (!_running || _asking) return;
    if (busy.value) {
      _skipped = true;
      return;
    }
    _asking = true;
    try {
      final answer = await source.list();
      if (!_running || !answer.ok) return;
      requests = [
        for (final r
            in ((answer.body['requests'] as List?) ?? const [])
                .cast<Map<String, dynamic>>())
          SignInRequest(r),
      ];
      notifyListeners();
    } finally {
      _asking = false;
    }
  }

  /// The next request not prompted for yet, now marked as prompted.
  SignInRequest? unprompted() {
    for (final r in requests) {
      if (!r.expired && _prompted.add(r.id)) return r;
    }
    return null;
  }

  /// One was answered here: it's gone from the list at once.
  void answered(String id) {
    requests = [
      for (final r in requests)
        if (r.id != id) r,
    ];
    notifyListeners();
  }
}

/// The app's: the vault starts and stops it (vault.dart).
final signInRequests = SignInRequests();

/// Shows a request: approved, denied, or left for later. Says what happened.
Future<void> reviewSignInRequest(
  BuildContext context,
  SignInRequest request, {
  String? email,
  SignInRequests? requests,
}) async {
  final list = requests ?? signInRequests;
  final done = await showDialog<String>(
    context: context,
    barrierDismissible: false,
    builder: (_) => SignInRequestDialog(
      request: request,
      email: email,
      source: list.source,
    ),
  );
  if (done == null || !context.mounted) return;
  if (done != 'expired') list.answered(request.id);
  ScaffoldMessenger.maybeOf(context)?.showSnackBar(
    SnackBar(
      content: Text(switch (done) {
        'approved' => 'Approved. ${request.device} is signing in.',
        'denied' => 'Denied. That sign-in won\'t go ahead.',
        _ => 'That sign-in request expired.',
      }),
      duration: const Duration(seconds: 4),
    ),
  );
}

/// The prompt: the fingerprint phrase first and largest, then the request's
/// details; Approve, Deny, or Later. Pops "approved", "denied", "expired",
/// or null for later.
class SignInRequestDialog extends StatefulWidget {
  const SignInRequestDialog({
    super.key,
    required this.request,
    required this.source,
    this.email,
  });

  final SignInRequest request;
  final SignInRequestSource source;
  final String? email;

  @override
  State<SignInRequestDialog> createState() => _SignInRequestDialogState();
}

class _SignInRequestDialogState extends State<SignInRequestDialog> {
  late final Timer ticker;
  bool working = false;
  String? error;

  @override
  void initState() {
    super.initState();
    // The time left, and closing when it's up.
    ticker = Timer.periodic(const Duration(seconds: 1), (_) {
      if (!mounted) return;
      if (widget.request.expired && !working) {
        Navigator.of(context).pop('expired');
        return;
      }
      setState(() {});
    });
  }

  @override
  void dispose() {
    ticker.cancel();
    super.dispose();
  }

  Future<void> answer(bool approve) async {
    if (working) return;
    setState(() {
      working = true;
      error = null;
    });
    final r = widget.request;
    if (approve && widget.source.confirmApprovals) {
      final confirmed = await widget.source.confirm(context);
      if (!mounted) return;
      if (!confirmed) {
        setState(() => working = false);
        return;
      }
    }
    var answer = await widget.source.answer(r.id, r.fingerprint, approve);
    // Confirmed too long ago (the prompt sat open): once more.
    if (approve && answer.body['confirmFirst'] == true && mounted) {
      if (await widget.source.confirm(context) && mounted) {
        answer = await widget.source.answer(r.id, r.fingerprint, approve);
      }
    }
    if (!mounted) return;
    if (answer.ok) {
      Navigator.of(context).pop(approve ? 'approved' : 'denied');
      return;
    }
    setState(() {
      working = false;
      error = answer.error ?? 'That didn\'t work. Try again.';
    });
  }

  @override
  Widget build(BuildContext context) {
    final r = widget.request;
    final colors = context.colors;
    final scheme = Theme.of(context).colorScheme;
    final email = widget.email;
    Widget detail(String label, String value) => Padding(
      padding: const EdgeInsets.only(top: 6),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 96,
            child: Text(label, style: TextStyle(color: colors.muted)),
          ),
          Expanded(child: Text(value)),
        ],
      ),
    );
    return AlertDialog(
      title: const Text('Approve sign-in?'),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                email == null
                    ? '${r.device} is asking to sign in to your account.'
                    : '${r.device} is asking to sign in as $email.',
              ),
              const SizedBox(height: 16),
              Text(
                'Fingerprint phrase',
                style: TextStyle(color: colors.muted, fontSize: 13),
              ),
              const SizedBox(height: 4),
              Container(
                key: const ValueKey('sign-in-fingerprint'),
                width: double.infinity,
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: scheme.primary.withValues(alpha: 0.08),
                  borderRadius: BorderRadius.circular(8),
                ),
                child: Text(
                  r.fingerprint,
                  style: const TextStyle(
                    fontSize: 20,
                    fontWeight: FontWeight.w600,
                    fontVariations: [FontVariation('wght', 600)],
                    height: 1.3,
                  ),
                ),
              ),
              const SizedBox(height: 8),
              const Text(
                'Approve only if this matches the phrase on the device '
                "you're signing in on, and you started that sign-in "
                'yourself, just now.',
              ),
              const SizedBox(height: 12),
              detail('Device', r.device),
              if (r.ip.isNotEmpty) detail('Address', r.ip),
              detail('Asked', askedAgo(r.age)),
              detail('Expires in', timeLeft(r.left)),
              if (error case final error?) ...[
                const SizedBox(height: 12),
                Text(error, style: TextStyle(color: scheme.error)),
              ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: working ? null : () => Navigator.of(context).pop(),
          child: const Text('Later'),
        ),
        OutlinedButton(
          onPressed: working ? null : () => answer(false),
          child: const Text('Deny'),
        ),
        WorkingButton(
          label: 'Approve',
          working: working,
          onPressed: () => answer(true),
        ),
      ],
    );
  }
}

/// The master password, to confirm it's the person approving a sign-in.
class _ConfirmWithMasterPassword extends StatefulWidget {
  const _ConfirmWithMasterPassword();

  @override
  State<_ConfirmWithMasterPassword> createState() =>
      _ConfirmWithMasterPasswordState();
}

class _ConfirmWithMasterPasswordState
    extends State<_ConfirmWithMasterPassword> {
  final password = TextEditingController();
  String? error;
  bool working = false;

  @override
  void dispose() {
    password.dispose();
    super.dispose();
  }

  Future<void> confirm() async {
    if (password.text.isEmpty || working) return;
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(
      () => Core.confirmWithMasterPassword(password.text),
    );
    if (!mounted) return;
    password.clear();
    if (answer.ok) {
      Navigator.of(context).pop(true);
      return;
    }
    setState(() {
      working = false;
      error = answer.error;
    });
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Confirm it\'s you'),
    content: SizedBox(
      width: 360,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('Enter your master password to approve this sign-in.'),
          const SizedBox(height: 12),
          TextField(
            controller: password,
            obscureText: true,
            autofocus: true,
            decoration: InputDecoration(
              labelText: 'Master password',
              errorText: error,
            ),
            onSubmitted: (_) => confirm(),
          ),
        ],
      ),
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.of(context).pop(false),
        child: const Text('Cancel'),
      ),
      WorkingButton(label: 'Continue', working: working, onPressed: confirm),
    ],
  );
}

/// Settings' group: the requests waiting, each to review, and whether
/// approving asks the person to confirm it's them.
class SignInRequestsSettings extends StatefulWidget {
  const SignInRequestsSettings({super.key, this.email, this.requests});

  final String? email;

  /// The app's ([signInRequests]) unless a test gives its own.
  final SignInRequests? requests;

  @override
  State<SignInRequestsSettings> createState() => _SignInRequestsSettingsState();
}

class _SignInRequestsSettingsState extends State<SignInRequestsSettings> {
  SignInRequests get requests => widget.requests ?? signInRequests;
  late bool confirm = requests.source.confirmApprovals;

  @override
  void initState() {
    super.initState();
    requests.addListener(changed);
    requests.check();
  }

  @override
  void dispose() {
    requests.removeListener(changed);
    super.dispose();
  }

  void changed() {
    if (mounted) setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final waiting = [
      for (final r in requests.requests)
        if (!r.expired) r,
    ];
    return SettingsGroup(
      title: 'Sign-in requests',
      footer:
          'When you choose "Log in with device" on the web vault or in the '
          'browser extension, approve it here.${isPhone ? ' Requests show while PassBundle is open.' : ''}',
      children: [
        if (waiting.isEmpty)
          const ListTile(title: Text('None waiting'))
        else
          for (final r in waiting)
            ListTile(
              key: ValueKey('sign-in-request-${r.id}'),
              title: Text(r.device),
              subtitle: Text(
                [
                  if (r.ip.isNotEmpty) r.ip,
                  askedAgo(r.age),
                  'expires in ${timeLeft(r.left).toLowerCase()}',
                ].join(' · '),
              ),
              trailing: TextButton(
                onPressed: () => reviewSignInRequest(
                  context,
                  r,
                  email: widget.email,
                  requests: requests,
                ),
                child: const Text('Review'),
              ),
            ),
        SwitchListTile(
          key: const ValueKey('confirm-approvals'),
          value: confirm,
          onChanged: (on) {
            requests.source.confirmApprovals = on;
            setState(() => confirm = requests.source.confirmApprovals);
          },
          title: const Text('Confirm before approving'),
          subtitle: Text(
            isPhone
                ? 'With your fingerprint or face, or your master password'
                : 'With Windows Hello, Touch ID, or your master password',
          ),
        ),
      ],
    );
  }
}
