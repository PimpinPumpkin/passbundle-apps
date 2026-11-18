// The nudges about two-step login above the vault's list (DESIGN.md in the
// meta repo, "A nudge for everyone else"; core/src/two_step_nudge.rs): to set
// it up, for someone with no two-step login at all, and to add a stronger
// method, for someone whose only second step is email codes. The server
// decides who sees which, only for people no rule requires it of, and the
// core asks it with each sync the person starts. A callout, never a dialog,
// in the web vault's and the extension's words. The apps can't set up
// two-step login themselves, so Set it up opens the web vault's two-step
// settings in the browser; Not now tells the server, which hides both for
// 90 days on every device. Offline, or from a server without it, nothing
// shows.
import 'package:flutter/material.dart';

/// The web vault's two-step login settings, after the server's address.
const webVaultTwoStepPath = '/#/settings/security/two-factor';

/// What each nudge says, as the web vault and the extension say it.
const twoStepNudgeWords = {
  'setUp':
      'Your password is all that protects your account. Add a second step, such as an authenticator app, so a stolen password isn\'t enough.',
  'stronger':
      'Codes by email are only as safe as your email account. An authenticator app or a security key is safer.',
};

/// The nudge a sync's answer names (`twoStepNudge`), or null: none, or one
/// this app doesn't know.
String? twoStepNudgeOf(Map<String, dynamic> answer) {
  final nudge = answer['twoStepNudge'];
  return nudge is String && twoStepNudgeWords.containsKey(nudge) ? nudge : null;
}

/// The callout above the list: the nudge's words, Not now, and Set it up.
class TwoStepNudgeBanner extends StatelessWidget {
  const TwoStepNudgeBanner({
    super.key,
    required this.nudge,
    required this.onSetUp,
    required this.onNotNow,
  });

  /// "setUp" or "stronger" ([twoStepNudgeWords]).
  final String nudge;
  final VoidCallback onSetUp;
  final VoidCallback onNotNow;

  @override
  Widget build(BuildContext context) => MaterialBanner(
    key: ValueKey('two-step-nudge-$nudge'),
    content: Text(twoStepNudgeWords[nudge] ?? ''),
    leading: const Icon(Icons.shield_outlined),
    actions: [
      TextButton(onPressed: onNotNow, child: const Text('Not now')),
      TextButton(onPressed: onSetUp, child: const Text('Set it up')),
    ],
  );
}
