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
//
// With several accounts signed in (accounts.dart), it asks for every
// unlocked one while a vault is shown, and each request is listed,
// confirmed, and answered for its own account, which the prompt names.
//
// Beside them, sign-ins on new devices that the new-device check holds
// (DESIGN.md in the meta repo, "New-device check"): the prompt shows the
// device, where it's from, and when, and takes the number the new device
// shows. A wrong number denies it. Approving sends no key, so it asks for no
// confirmation first: the number is the proof. "Not now" brings it back in
// 2 minutes if it's still waiting.
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

/// A sign-in waiting for an account's answer, as the core listed it.
class SignInRequest {
  SignInRequest(Map<String, dynamic> json, {DateTime? listedAt})
    : account = json['account'] as String?,
      email = json['email'] as String?,
      id = json['id'] as String,
      deviceId = json['deviceId'] as String? ?? '',
      deviceType = json['deviceType'] as String? ?? '',
      device = json['device'] as String? ?? 'A device',
      ip = json['ip'] as String? ?? '',
      fingerprint = json['fingerprint'] as String,
      _age = Duration(seconds: (json['ageSeconds'] as num?)?.toInt() ?? 0),
      _left = Duration(seconds: (json['secondsLeft'] as num?)?.toInt() ?? 0),
      _listedAt = listedAt ?? signInClock();

  /// The account it's for, by its id on this device, and its email: the
  /// one it was listed for, which alone answers it. Null for the account
  /// shown.
  final String? account;
  final String? email;

  final String id;

  /// The requesting device's identifier (empty from a server that doesn't
  /// say).
  final String deviceId;

  /// Its type, as the server names it ("Chrome").
  final String deviceType;

  /// Which device asked, for prompting once per device and for showing a
  /// request it sent again in its place: its identifier, type, and address
  /// together (a request from the same identifier as another type, or from
  /// another address, is another asker), or the request where there's no
  /// identifier.
  String get asker =>
      deviceId.isEmpty ? id : '${account ?? ''} $deviceId $deviceType $ip';

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

/// A sign-in on a new device that the new-device check holds, waiting for
/// an account's approval, as the core listed it. Never its number: only
/// the new device shows that.
class NewDeviceCheck {
  NewDeviceCheck(Map<String, dynamic> json, {DateTime? listedAt})
    : account = json['account'] as String?,
      email = json['email'] as String?,
      id = json['id'] as String,
      device = json['device'] as String? ?? 'A device',
      ip = json['ip'] as String? ?? '',
      location = json['location'] as String?,
      sourceName = (json['locationSource'] as Map?)?['name'] as String?,
      sourceUrl = (json['locationSource'] as Map?)?['url'] as String?,
      emailAfter = json['emailAfter'] as String?,
      _age = Duration(seconds: (json['ageSeconds'] as num?)?.toInt() ?? 0),
      _left = Duration(seconds: (json['secondsLeft'] as num?)?.toInt() ?? 0),
      _listedAt = listedAt ?? signInClock();

  /// The account it's for, by its id on this device, and its email. Null
  /// for the account shown.
  final String? account;
  final String? email;
  final String id;

  /// The device, in the server's words ("Web vault in Firefox").
  final String device;

  /// Its address, as the server saw it.
  final String ip;

  /// Roughly where it is ("Berlin, Germany"), or null when the server
  /// doesn't know.
  final String? location;

  /// Who the location is from, to credit beside it, and their address.
  final String? sourceName;
  final String? sourceUrl;

  /// When a code by email may be sent for it, if someone asked for one
  /// after the wait.
  final String? emailAfter;
  final Duration _age;
  final Duration _left;
  final DateTime _listedAt;

  /// Where it's from, with its address: "Berlin, Germany (203.0.113.7)".
  String get where {
    final place = location ?? 'Unknown location';
    return ip.isEmpty ? place : '$place ($ip)';
  }

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

  /// Approves or denies one, for its [account] (the one shown when null);
  /// approving may answer `confirmFirst`.
  Future<Answer> answer(
    String id,
    String fingerprint,
    bool approve, [
    String? account,
  ]);

  /// Asks the person to confirm it's them, for an [account] (the one shown
  /// when null): biometrics, else the master password. True once they have.
  Future<bool> confirm(BuildContext context, [String? account]);

  /// Whether this device asks them to before approving.
  bool get confirmApprovals;

  /// Turning it off needs a confirmation just now (`confirmFirst`).
  Answer setConfirmApprovals(bool on);

  /// Approves a sign-in the new-device check holds with the [number] typed,
  /// or denies it (null), for its [account] (the one shown when null):
  /// `answered` is "approved", "denied", or "wrongNumber".
  Future<Answer> answerCheck(String id, String? number, [String? account]);
}

/// The core's: every unlocked account's requests, the shown one's first,
/// each with its account.
class CoreSignInRequests implements SignInRequestSource {
  @override
  Future<Answer> list() async {
    final shown = Core.account;
    final unlocked = [
      for (final a in (Core().accounts()['accounts'] as List? ?? const []))
        if ((a as Map)['state'] == 'unlocked')
          (a['id'] as String, a['email'] as String?),
    ]..sort((a, b) => (a.$1 == shown ? 0 : 1) - (b.$1 == shown ? 0 : 1));
    final requests = <Map<String, dynamic>>[];
    final checks = <Map<String, dynamic>>[];
    Answer? failed;
    var listed = false;
    for (final (id, email) in unlocked) {
      final answer = await Core.signInRequests(id);
      if (!answer.ok) {
        failed ??= answer;
        continue;
      }
      listed = true;
      for (final r in (answer.body['requests'] as List? ?? const [])) {
        requests.add({
          ...Map<String, dynamic>.from(r as Map),
          'account': id,
          'email': ?email,
        });
      }
      for (final c in (answer.body['checks'] as List? ?? const [])) {
        checks.add({
          ...Map<String, dynamic>.from(c as Map),
          'account': id,
          'email': ?email,
          'locationSource': answer.body['locationSource'],
        });
      }
    }
    if (!listed && failed != null) return failed;
    return Answer({'ok': true, 'requests': requests, 'checks': checks});
  }

  @override
  Future<Answer> answerCheck(String id, String? number, [String? account]) =>
      whileBusy(() => Core.answerNewDeviceCheck(id, number, account));

  @override
  Future<Answer> answer(
    String id,
    String fingerprint,
    bool approve, [
    String? account,
  ]) => whileBusy(
    () => Core.answerSignInRequest(id, fingerprint, approve, account),
  );

  @override
  Future<bool> confirm(BuildContext context, [String? account]) async {
    final status = await Core.biometricStatus(account);
    if (status['status'] == 'setUp') {
      final answer = await whileBusy(
        () => Core.confirmApprovalWithBiometric(account),
      );
      if (answer.ok && answer.body['confirmed'] == true) return true;
    }
    if (!context.mounted) return false;
    return await showDialog<bool>(
          context: context,
          builder: (_) => _ConfirmWithMasterPassword(account: account),
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

  /// Sign-ins on new devices waiting for a number.
  List<NewDeviceCheck> checks = const [];
  final _prompted = <String>{};

  /// When each check was last prompted for.
  final _promptedChecks = <String, DateTime>{};

  /// How long "Not now" puts a check off.
  static const checkAgainAfter = Duration(minutes: 2);

  /// When each requesting device was last prompted for.
  final _promptedDevices = <String, DateTime>{};

  /// How long a device isn't prompted for again: a request's lifetime.
  static const promptEvery = Duration(minutes: 15);
  Timer? _timer;
  bool _asking = false;
  bool _running = false;

  /// A check skipped beside a slow call, made as soon as it's done.
  bool _skipped = false;

  /// Counts each stop: a listing asked for before one (for the account the
  /// vault showed then: the screens may show another now) is dropped.
  int _session = 0;

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

  /// The vault closed (it locked, or the screens show another account):
  /// forgets every request.
  void stop() {
    if (_running) busy.removeListener(_busyChanged);
    _running = false;
    _session++;
    _skipped = false;
    pause();
    requests = const [];
    checks = const [];
    _prompted.clear();
    _promptedDevices.clear();
    _promptedChecks.clear();
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
    final session = _session;
    try {
      final answer = await source.list();
      if (!_running || session != _session || !answer.ok) return;
      requests = [
        for (final r
            in ((answer.body['requests'] as List?) ?? const [])
                .cast<Map<String, dynamic>>())
          SignInRequest(r),
      ];
      checks = [
        for (final c
            in ((answer.body['checks'] as List?) ?? const [])
                .cast<Map<String, dynamic>>())
          NewDeviceCheck(c),
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

  /// The request that took this one's place: still waiting, or a newer one
  /// from the same device (identifier, type, and address all the same: it
  /// sent it again), or none. A new request that differs in any of them
  /// isn't this one's: the open prompt closes, and it prompts on its own.
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
    checks = [
      for (final c in checks)
        if (c.id != id) c,
    ];
    notifyListeners();
  }

  /// The next new device's check to prompt for, now marked as prompted:
  /// one not prompted for in the last 2 minutes ("Not now" puts it off
  /// that long).
  NewDeviceCheck? unpromptedCheck() {
    final now = signInClock();
    for (final c in checks) {
      if (c.expired) continue;
      final last = _promptedChecks[c.id];
      if (last != null && now.difference(last) < checkAgainAfter) continue;
      _promptedChecks[c.id] = now;
      return c;
    }
    return null;
  }

  /// The check as listed now, or null once it's gone (answered elsewhere,
  /// or ended).
  NewDeviceCheck? stillWaiting(NewDeviceCheck check) {
    for (final c in checks) {
      if (c.id == check.id && !c.expired) return c;
    }
    return null;
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

/// Shows a sign-in the new-device check holds: approved with the number,
/// denied, or left for later ("Not now"). Says what happened.
Future<void> reviewNewDeviceCheck(
  BuildContext context,
  NewDeviceCheck check, {
  String? email,
  SignInRequests? requests,
}) async {
  final list = requests ?? signInRequests;
  final done = await showDialog<(String, NewDeviceCheck)>(
    context: context,
    barrierDismissible: false,
    builder: (_) =>
        NewDeviceCheckDialog(check: check, email: email, requests: list),
  );
  if (done == null || !context.mounted) return;
  final (what, shown) = done;
  if (what == 'approved' || what == 'denied' || what == 'wrongNumber') {
    list.answered(shown.id);
  }
  ScaffoldMessenger.maybeOf(context)?.showSnackBar(
    SnackBar(
      content: Text(switch (what) {
        'approved' => 'Approved. ${shown.device} is signing in.',
        'denied' => 'Denied. That sign-in won\'t go ahead.',
        'wrongNumber' =>
          'That wasn\'t the number on the new device, so the sign-in was denied. If it was you, sign in again there.',
        'gone' => 'That sign-in was answered on another device, or it ended.',
        _ => 'That sign-in ended.',
      }),
      duration: const Duration(seconds: 5),
    ),
  );
}

/// The prompt for a sign-in on a new device: who's signing in, the device,
/// where it's from, and when, then the number the new device shows.
/// Approve sends the number (a wrong one denies it), Deny denies it, and
/// "Not now" leaves it waiting. Pops what happened ("approved", "denied",
/// "wrongNumber", "expired", "gone") and the check shown, or null for not
/// now.
class NewDeviceCheckDialog extends StatefulWidget {
  const NewDeviceCheckDialog({
    super.key,
    required this.check,
    required this.requests,
    this.email,
  });

  final NewDeviceCheck check;
  final SignInRequests requests;
  final String? email;

  @override
  State<NewDeviceCheckDialog> createState() => _NewDeviceCheckDialogState();
}

class _NewDeviceCheckDialogState extends State<NewDeviceCheckDialog> {
  late final Timer ticker;
  late NewDeviceCheck check = widget.check;
  final number = TextEditingController();
  bool working = false;
  String? error;

  SignInRequestSource get source => widget.requests.source;

  /// Two digits typed: Approve can send them.
  bool get typed => RegExp(r'^[0-9]{2}$').hasMatch(number.text.trim());

  @override
  void initState() {
    super.initState();
    widget.requests.addListener(listed);
    number.addListener(() {
      if (mounted) setState(() {});
    });
    ticker = Timer.periodic(const Duration(seconds: 1), (_) {
      if (!mounted) return;
      if (check.expired && !working) {
        Navigator.of(context).pop(('expired', check));
        return;
      }
      setState(() {});
    });
  }

  @override
  void dispose() {
    widget.requests.removeListener(listed);
    ticker.cancel();
    number.dispose();
    super.dispose();
  }

  /// The list changed: still waiting, or answered elsewhere or ended.
  void listed() {
    if (!mounted || working) return;
    final now = widget.requests.stillWaiting(check);
    if (now == null) {
      Navigator.of(context).pop(('gone', check));
      return;
    }
    setState(() => check = now);
  }

  Future<void> answer(bool approve) async {
    if (working || (approve && !typed)) return;
    setState(() {
      working = true;
      error = null;
    });
    final c = check;
    final answer = await source.answerCheck(
      c.id,
      approve ? number.text.trim() : null,
      c.account,
    );
    if (!mounted) return;
    if (answer.ok) {
      Navigator.of(context).pop((answer.body['answered'] as String? ?? 'denied', c));
      return;
    }
    setState(() {
      working = false;
      error = answer.error ?? 'That didn\'t work. Try again.';
    });
  }

  @override
  Widget build(BuildContext context) {
    final c = check;
    final colors = context.colors;
    final scheme = Theme.of(context).colorScheme;
    // The account it's for, which may not be the one shown.
    final email = c.email ?? widget.email;
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
    final credit = c.sourceName;
    final url = c.sourceUrl;
    return AlertDialog(
      title: const Text('Approve this sign-in?'),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Someone is signing in to ${email ?? 'your account'} on a new device. '
                'Approve only if it\'s you, on a device in front of you.',
              ),
              const SizedBox(height: 12),
              detail('Device', c.device),
              detail('Where', c.where),
              detail('When', askedAgo(c.age)),
              detail('Expires in', timeLeft(c.left)),
              // Who the location is from, credited beside it.
              if (c.location != null && credit != null)
                Padding(
                  padding: const EdgeInsets.only(top: 6, left: 96),
                  child: InkWell(
                    key: const ValueKey('location-source'),
                    onTap: url == null ? null : () => Core.openWebsite(url),
                    child: Text(
                      credit,
                      style: TextStyle(
                        color: url == null ? colors.muted : scheme.primary,
                        fontSize: 12,
                        decoration: url == null
                            ? null
                            : TextDecoration.underline,
                      ),
                    ),
                  ),
                ),
              if (c.emailAfter != null) ...[
                const SizedBox(height: 12),
                Text(
                  'Someone asked for a code by email for this sign-in instead. If it isn\'t you, deny it.',
                  style: TextStyle(
                    color: scheme.error,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ],
              const SizedBox(height: 16),
              TextField(
                key: const ValueKey('new-device-check-number'),
                controller: number,
                enabled: !working,
                autofocus: !isPhone,
                keyboardType: TextInputType.number,
                autocorrect: false,
                enableSuggestions: false,
                maxLength: 2,
                inputFormatters: [FilteringTextInputFormatter.digitsOnly],
                onSubmitted: (_) => answer(true),
                decoration: InputDecoration(
                  labelText: 'Number shown on the new device',
                  helperText: 'Two digits. A wrong number denies the sign-in.',
                  helperMaxLines: 2,
                  counterText: '',
                  errorText: error,
                  errorMaxLines: 4,
                ),
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: working ? null : () => Navigator.of(context).pop(),
          child: const Text('Not now'),
        ),
        OutlinedButton(
          onPressed: working ? null : () => answer(false),
          child: const Text('Deny'),
        ),
        FilledButton(
          onPressed: working || !typed ? null : () => answer(true),
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
      final confirmed = await source.confirm(context, r.account);
      if (!mounted) return;
      if (!confirmed) {
        setState(() => working = false);
        return;
      }
    }
    var answer = await source.answer(r.id, r.fingerprint, approve, r.account);
    // Confirmed too long ago (the prompt sat open), or the request's account
    // asks to confirm where the one shown doesn't: once more.
    if (approve && answer.body['confirmFirst'] == true && mounted) {
      if (await source.confirm(context, r.account) && mounted) {
        answer = await source.answer(r.id, r.fingerprint, approve, r.account);
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
    // The account it's for, which may not be the one shown.
    final email = r.email ?? widget.email;
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
  const _ConfirmWithMasterPassword({this.account});

  /// The account whose master password it is (the one shown when null).
  final String? account;

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
      () => Core.confirmWithMasterPassword(password.text, widget.account),
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
  const SignInRequestsSettings({
    super.key,
    this.email,
    this.requests,
    this.touchIdOn = false,
  });

  final String? email;

  /// On the Mac, whether unlocking with Touch ID is on for this account,
  /// which is what confirming asks for when it is (Settings knows).
  final bool touchIdOn;

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
    final checks = [
      for (final c in requests.checks)
        if (!c.expired) c,
    ];
    return SettingsGroup(
      title: 'Sign-in requests',
      footer:
          note ??
          'When you sign in on a new device, or choose "Log in with device" '
              'on the web vault or in the browser extension, approve it here.${isPhone ? ' Requests show while PassBundle is open.' : ''}',
      footerColor: note == null ? null : Theme.of(context).colorScheme.error,
      children: [
        if (waiting.isEmpty && checks.isEmpty)
          const ListTile(title: Text('None waiting')),
        for (final c in checks)
          ListTile(
            key: ValueKey('new-device-check-${c.id}'),
            title: Text(c.device),
            subtitle: Text(
              [
                // Another account's: which one.
                if (c.email case final email? when email != widget.email)
                  email,
                'New device',
                c.where,
                askedAgo(c.age),
              ].join(' · '),
            ),
            trailing: TextButton(
              onPressed: () => reviewNewDeviceCheck(
                context,
                c,
                email: widget.email,
                requests: requests,
              ),
              child: const Text('Review'),
            ),
          ),
        for (final r in waiting)
          ListTile(
            key: ValueKey('sign-in-request-${r.id}'),
            title: Text(r.device),
            subtitle: Text(
              [
                // Another account's: which one.
                if (r.email case final email? when email != widget.email)
                  email,
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
          // How it asks here: the phone's biometrics, the Mac's Touch ID
          // once it's on, Windows Hello, or the master password. Whether
          // it's the Mac's way is asked before what this computer is: the
          // Mac's tests run on Windows too.
          subtitle: Text(
            isPhone
                ? 'With your fingerprint or face, or your master password'
                : isMac && widget.touchIdOn
                ? 'With Touch ID or your master password'
                : !isMac && Platform.isWindows
                ? 'With Windows Hello or your master password'
                : 'With your master password',
          ),
        ),
      ],
    );
  }
}
