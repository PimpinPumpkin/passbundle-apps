// Push for sign-in requests while PassBundle is closed, on Android with a
// UnifiedPush distributor app such as ntfy (DESIGN.md in the meta repo, "Push
// to phones"; docs/phones.md, "Approving a sign-in"). The phone's own code
// does it (Push.kt): it registers each account on the phone with the
// distributor, gives the account's server the endpoint, and when a message
// says a sign-in waits, asks the server (through the core) and shows one
// notification in the prompt's words. The screens only tell it which
// accounts are signed in and unlocked, say before signing out of one, and
// show in Settings whether it's on. Without a distributor nothing changes:
// the app asks while it's open, as before.
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';

/// A call to the phone's own code (MainActivity.kt), or null where it
/// doesn't answer: not Android, or a test's stand-in without it.
Future<T?> _onAndroid<T>(String method, [Object? arguments]) async {
  if (!isAndroid) return null;
  try {
    return await Core.platform.invokeMethod<T>(method, arguments);
  } on PlatformException {
    return null;
  } on MissingPluginException {
    return null;
  }
}

/// The accounts signed in, and those unlocked, by their ids on this device,
/// as the phone was last told: told again only when they change.
String? _told;

/// Tests only: the phone is told again, as after the app starts.
@visibleForTesting
void forgetPushTold() => _told = null;

/// After signing in, unlocking, or signing out (main.dart): the phone
/// registers the unlocked accounts with the distributor where it needs to,
/// and drops the ones no longer signed in. It asks each account's server
/// itself, off the screens.
Future<void> syncPush({
  required List<String> signedIn,
  required List<String> unlocked,
}) async {
  final told = '${signedIn.join(',')}|${unlocked.join(',')}';
  if (!isAndroid || told == _told) return;
  _told = told;
  await _onAndroid<Object>('pushSync', {
    'signedIn': signedIn,
    'unlocked': unlocked,
  });
}

/// Before signing out of an account: its endpoint goes from its server while
/// its session still works, then from the distributor. Never holds up
/// signing out for long.
Future<void> forgetPush(String account) async {
  if (!isAndroid || account.isEmpty) return;
  _told = null;
  try {
    await _onAndroid<Object>('pushForget', {
      'account': account,
    }).timeout(const Duration(seconds: 5));
  } on TimeoutException {
    // The phone goes on with it; signing out doesn't wait.
  }
}

/// What the phone says about push for an account.
@immutable
class PushStatus {
  const PushStatus({
    this.installed = false,
    this.distributor,
    this.registered = false,
  });

  PushStatus.from(Map<Object?, Object?> answer)
    : installed = answer['installed'] == true,
      distributor = answer['distributor'] as String?,
      registered = answer['registered'] == true;

  /// Whether any distributor app is on the phone.
  final bool installed;

  /// The one in use, by the name the phone shows for it.
  final String? distributor;

  /// Whether this account is registered with it.
  final bool registered;
}

/// What the phone says for [account], or null where it doesn't (not Android).
Future<PushStatus?> pushStatus(String account) async {
  final answer = await _onAndroid<Map>('pushStatus', {'account': account});
  return answer == null ? null : PushStatus.from(answer);
}

/// Settings' line about it.
const pushOffLine =
    'Off. Install a UnifiedPush app, such as ntfy, and sign-in requests reach '
    'PassBundle while it\'s closed.';

String pushLine(PushStatus status) {
  if (!status.installed) return pushOffLine;
  final name = status.distributor ?? 'your UnifiedPush app';
  return status.registered
      ? 'On, through $name.'
      : 'Off for this account: its server doesn\'t send them, or $name '
            'hasn\'t answered yet.';
}

/// Settings' row: whether sign-in requests reach PassBundle while it's
/// closed, and through which distributor, or how to make them. Android only;
/// asked again when the app is back in front (a distributor installed
/// meanwhile, say).
class PushSetting extends StatefulWidget {
  const PushSetting({super.key, required this.account});

  final String account;

  @override
  State<PushSetting> createState() => _PushSettingState();
}

class _PushSettingState extends State<PushSetting> {
  PushStatus? status;

  late final AppLifecycleListener lifecycle = AppLifecycleListener(
    onResume: refresh,
  );

  @override
  void initState() {
    super.initState();
    lifecycle;
    refresh();
  }

  @override
  void dispose() {
    lifecycle.dispose();
    super.dispose();
  }

  Future<void> refresh() async {
    final next = await pushStatus(widget.account);
    if (mounted) setState(() => status = next);
  }

  @override
  Widget build(BuildContext context) {
    final status = this.status;
    if (status == null) return const SizedBox.shrink();
    return ListTile(
      key: const ValueKey('push-to-phone'),
      leading: Icon(
        status.registered
            ? Icons.notifications_active_outlined
            : Icons.notifications_off_outlined,
      ),
      title: const Text('Notifications while closed'),
      subtitle: Text(pushLine(status)),
    );
  }
}
