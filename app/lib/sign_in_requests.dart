// Approving a sign-in from another device (docs/core.md, "Approving a
// sign-in"; DESIGN.md in the meta repo): someone signing in on the web vault
// or the browser extension picks "Log in with device", and approves it here.
// While the vault is open and the app is on screen, it asks the server every
// 10 seconds for sign-ins waiting for this account's answer, and prompts over
// the vault once per requesting device every 15 minutes (a request sent again
// shows in the open prompt's place, with its new phrase); Settings lists them
// all. The core makes the fingerprint phrase, wraps the key, and refuses an
// approval, or turning off confirming, that the person didn't confirm; this
// only draws, and asks.
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
      deviceId = json['deviceId'] as String? ?? '',
      device = json['device'] as String? ?? 'A device',
      ip = json['ip'] as String? ?? '',
      fingerprint = json['fingerprint'] as String,
      _age = Duration(seconds: (json['ageSeconds'] as num?)?.toInt() ?? 0),
      _left = Duration(seconds: (json['secondsLeft'] as num?)?.toInt() ?? 0),
      _listedAt = listedAt ?? signInClock();

  final String id;

  /// The requesting device's identifier (empty from a server that doesn't
  /// say).
  final String deviceId;

  /// Which device asked, for prompting once per device: its identifier, or
  /// the request where there's none.
  String get asker => deviceId.isEmpty ? id : deviceId;

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

  /// Turning it off needs a confirmation just now (`confirmFirst`).
  Answer setConfirmApprovals(bool on);
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
      final answer = await whileBusy(Core.confirmApprovalWithBiometric);
      if (answer.ok && answer.body['confirmed'] == true) return true;
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
  Answer setConfirmApprovals(bool on) => Core().setConfirmApprovals(on);
}

/// The requests waiting, asked for every 10 seconds while the vault is open
/// and the app is on screen ([start], [pause], [resume], [stop]). Each
/// requesting device is prompted for once every 15 minutes ([unprompted]),
/// so sending again ("Resend") can't keep the prompts coming.
class SignInRequests extends ChangeNotifier {
  SignInRequests([SignInRequestSource? source])
    : source = source ?? CoreSignInRequests();

  SignInRequestSource source;
  List<SignInRequest> requests = const [];
  final _prompted = <String>{};

  /// When each requesting device was last prompted for.
  final _promptedDevices = <String, DateTime>{};

  /// How long a device isn't prompted for again: a request's lifetime.
  static const promptEvery = Duration(minutes: 15);
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
    _promptedDevices.clear();
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

  /// The next request to prompt for, now marked as prompted: one not
  /// prompted for yet, from a device not prompted for in 15 minutes. Another
  /// from such a device waits in Settings (or shows in its device's open
  /// prompt, [replacing]).
  SignInRequest? unprompted() {
    final now = signInClock();
    for (final r in requests) {
      if (r.expired || !_prompted.add(r.id)) continue;
      final last = _promptedDevices[r.asker];
      if (last != null && now.difference(last) < promptEvery) continue;
      _promptedDevices[r.asker] = now;
      return r;
    }
    return null;
  }

  /// The request that took this one's place: still waiting, or the same
  /// device's newer one (it sent it again), or none.
  SignInRequest? replacing(SignInRequest request) {
    for (final r in requests) {
      if (r.id == request.id) return r;
    }
    for (final r in requests) {
      if (r.asker == request.asker && !r.expired) return r;
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
  final done = await showDialog<(String, SignInRequest)>(
    context: context,
    barrierDismissible: false,
    builder: (_) =>
        SignInRequestDialog(request: request, email: email, requests: list),
  );
  if (done == null || !context.mounted) return;
  final (what, shown) = done;
  if (what == 'approved' || what == 'denied') list.answered(shown.id);
  ScaffoldMessenger.maybeOf(context)?.showSnackBar(
    SnackBar(
      content: Text(switch (what) {
        'approved' => 'Approved. ${shown.device} is signing in.',
        'denied' => 'Denied. That sign-in won\'t go ahead.',
        'gone' => 'That sign-in request was answered elsewhere, or expired.',
        _ => 'That sign-in request expired.',
      }),
      duration: const Duration(seconds: 4),
    ),
  );
}

/// The prompt: the fingerprint phrase first and largest, then the request's
/// details; Approve, Deny, or Later. If the device sends its request again,
/// the new one takes its place here, with its new phrase, and Approve waits
/// a moment so it isn't tapped for the old one. Pops what happened
/// ("approved", "denied", "expired", "gone") and the request shown, or null
/// for later.
class SignInRequestDialog extends StatefulWidget {
  const SignInRequestDialog({
    super.key,
    required this.request,
    required this.requests,
    this.email,
  });

  final SignInRequest request;
  final SignInRequests requests;
  final String? email;

  @override
  State<SignInRequestDialog> createState() => _SignInRequestDialogState();
}

class _SignInRequestDialogState extends State<SignInRequestDialog> {
  late final Timer ticker;
  late SignInRequest request = widget.request;
  bool working = false;
  String? error;

  /// The device sent it again: the phrase shown is the new request's.
  bool replaced = false;
  DateTime? holdApproveUntil;

  /// How long Approve waits after the request changed under it.
  static const hold = Duration(seconds: 3);

  SignInRequestSource get source => widget.requests.source;

  @override
  void initState() {
    super.initState();
    widget.requests.addListener(listed);
    // The time left, and closing when it's up.
    ticker = Timer.periodic(const Duration(seconds: 1), (_) {
      if (!mounted) return;
      if (request.expired && !working) {
        Navigator.of(context).pop(('expired', request));
        return;
      }
      setState(() {});
    });
  }

  @override
  void dispose() {
    widget.requests.removeListener(listed);
    ticker.cancel();
    super.dispose();
  }

  /// The list changed: the same request, the device's new one, or none.
  void listed() {
    if (!mounted || working) return;
    final next = widget.requests.replacing(request);
    if (next == null) {
      Navigator.of(context).pop(('gone', request));
      return;
    }
    if (next.id == request.id) return;
    setState(() {
      request = next;
      replaced = true;
      error = null;
      holdApproveUntil = signInClock().add(hold);
    });
  }

  bool get holding {
    final until = holdApproveUntil;
    return until != null && signInClock().isBefore(until);
  }

  Future<void> answer(bool approve) async {
    if (working || (approve && holding)) return;
    setState(() {
      working = true;
      error = null;
    });
    final r = request;
    if (approve && source.confirmApprovals) {
      final confirmed = await source.confirm(context);
      if (!mounted) return;
      if (!confirmed) {
        setState(() => working = false);
        return;
      }
    }
    var answer = await source.answer(r.id, r.fingerprint, approve);
    // Confirmed too long ago (the prompt sat open): once more.
    if (approve && answer.body['confirmFirst'] == true && mounted) {
      if (await source.confirm(context) && mounted) {
        answer = await source.answer(r.id, r.fingerprint, approve);
      }
    }
    if (!mounted) return;
    if (answer.ok) {
      Navigator.of(context).pop((approve ? 'approved' : 'denied', r));
      return;
    }
    setState(() {
      working = false;
      error = answer.error ?? 'That didn\'t work. Try again.';
    });
  }

  @override
  Widget build(BuildContext context) {
    final r = request;
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
              if (replaced) ...[
                const SizedBox(height: 8),
                Text(
                  'Sent again from the same device. The phrase is new: '
                  'compare it again.',
                  key: const ValueKey('sign-in-replaced'),
                  style: TextStyle(
                    color: scheme.primary,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ],
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
        FilledButton(
          onPressed: working || holding ? null : () => answer(true),
          child: working
              ? const SizedBox(
                  width: 18,
                  height: 18,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Text('Approve'),
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

  /// Why turning confirming off didn't work.
  String? note;

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
          note ??
          'When you choose "Log in with device" on the web vault or in the '
              'browser extension, approve it here.${isPhone ? ' Requests show while PassBundle is open.' : ''}',
      footerColor: note == null ? null : Theme.of(context).colorScheme.error,
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
          onChanged: (on) async {
            // Turning it off asks the person to confirm it's them first,
            // and the core holds to that.
            if (!on && !await requests.source.confirm(context)) return;
            final answer = requests.source.setConfirmApprovals(on);
            if (!mounted) return;
            setState(() {
              confirm = requests.source.confirmApprovals;
              note = answer.ok ? null : answer.error;
            });
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
