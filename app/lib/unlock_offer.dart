// Unlocking with biometrics, offered rather than left for people to find
// (docs/phones.md, "Unlocking with biometrics"). After someone signs in or
// unlocks with their master password on a phone with a fingerprint, a face,
// Face ID, or Touch ID set up, and unlocking with it isn't on yet, a sheet
// asks once whether to use it next time. "Turn on" is what Settings' switch
// does: the phone asks for the biometric, and only once the person gives it
// is the user key sealed behind it (Biometric.kt, AppDelegate.swift). Nothing
// turns on without that. "Not now", closing the sheet, or turning it off in
// Settings isn't asked again for 30 days, and an organization's "Remove
// Unlock with PIN" policy means it isn't offered at all. Settings keeps the
// switch either way. Phones only: on a computer, Windows Hello and Touch ID
// are shared with the browser extension, and stay a choice in Settings.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

/// How long "Not now" holds.
const offerAgainAfter = Duration(days: 30);

/// Tests only: the clock the offer goes by.
@visibleForTesting
DateTime Function() offerClock = DateTime.now;

/// Where "Not now" is remembered: beside the vault, in the app's own folder.
File? get _declined {
  final dir = Core.folder;
  return dir == null ? null : File('$dir/biometric-offer.json');
}

/// When the person last said "Not now" here, or null.
DateTime? offerDeclinedAt() {
  try {
    final json =
        jsonDecode(_declined!.readAsStringSync()) as Map<String, dynamic>;
    return DateTime.tryParse(json['declinedAt'] as String? ?? '');
  } on Object {
    return null;
  }
}

/// "Not now": not asked again for 30 days.
void declineOffer() {
  try {
    _declined?.writeAsStringSync(
      jsonEncode({'declinedAt': offerClock().toUtc().toIso8601String()}),
    );
  } on FileSystemException {
    // Then it's asked again after the next unlock.
  }
}

/// Whether it may be offered now, as far as this device knows without asking
/// the phone: on a phone, not declined in the last 30 days, and with no
/// organization's policy against it. It reads the core, so it's asked before
/// the vault's first sync takes the core.
bool mayOfferBiometric(Core core) {
  if (!isPhone) return false;
  final declined = offerDeclinedAt();
  if (declined != null && offerClock().difference(declined) < offerAgainAfter) {
    return false;
  }
  return core.lockPolicy()['removeUnlockWithPin'] != true;
}

/// Asks the phone whether it has biometrics set up, and unlocking with them
/// is off here; then offers it. Its name ("fingerprint") if it's on now.
Future<String?> offerBiometricUnlock(BuildContext context) async {
  final status = await Core.biometricStatus();
  if (status['status'] != 'notSetUp' || !context.mounted) return null;
  final method = (status['method'] as String?) ?? 'biometrics';
  final on = await showModalBottomSheet<bool>(
    context: context,
    isScrollControlled: true,
    builder: (_) => BiometricOffer(method: method),
  );
  if (on != true) declineOffer();
  return on == true ? method : null;
}

/// Turns biometric unlock on: the phone (or the desktop agent) asks for the
/// biometric, and only then seals the user key behind it. `on` in the answer
/// says whether the person gave it. A slow call already running (the sync
/// after signing in, say) has the core, and sealing needs the user key from
/// it: this waits for that call, or the core would answer that PassBundle is
/// locked.
Future<Answer> turnOnBiometric() async {
  for (var i = 0; i < 600 && busy.value; i++) {
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
  return whileBusy(Core.enrollBiometric);
}

/// "your fingerprint", "Face ID": the phone's word for it, in a sentence.
String yourMethod(String method) => switch (method) {
  'fingerprint' || 'biometrics' => 'your $method',
  _ => method,
};

/// The sheet: what it is, "Turn on", and "Not now".
class BiometricOffer extends StatefulWidget {
  const BiometricOffer({super.key, required this.method});

  /// As the phone calls it: "fingerprint", "face unlock", "Face ID", "Touch ID".
  final String method;

  @override
  State<BiometricOffer> createState() => _BiometricOfferState();
}

class _BiometricOfferState extends State<BiometricOffer> {
  bool working = false;
  String? note;

  Future<void> turnOn() async {
    setState(() {
      working = true;
      note = null;
    });
    final answer = await turnOnBiometric();
    if (!mounted) return;
    if (answer.ok && answer.body['on'] == true) {
      Navigator.of(context).pop(true);
      return;
    }
    setState(() {
      working = false;
      note = answer.ok
          ? 'It wasn\'t turned on: ${widget.method} was cancelled.'
          : answer.error;
    });
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final scheme = Theme.of(context).colorScheme;
    final face = widget.method.toLowerCase().contains('face');
    return PopScope(
      // Not while the phone is asking for the biometric.
      canPop: !working,
      child: SafeArea(
        child: SingleChildScrollView(
          padding: const EdgeInsets.fromLTRB(24, 0, 24, 16),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Center(
                child: Container(
                  width: 64,
                  height: 64,
                  decoration: BoxDecoration(
                    color: c.tint,
                    shape: BoxShape.circle,
                  ),
                  child: Icon(
                    face
                        ? Icons.face_unlock_outlined
                        : Icons.fingerprint_rounded,
                    size: 34,
                    color: scheme.primary,
                  ),
                ),
              ),
              const SizedBox(height: 16),
              Text(
                widget.method == 'face unlock'
                    ? 'Use face unlock next time?'
                    : 'Unlock with ${widget.method} next time?',
                textAlign: TextAlign.center,
                style: const TextStyle(
                  fontSize: 22,
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(height: 8),
              Text(
                'Open PassBundle with ${yourMethod(widget.method)} instead of '
                'typing your master password. You can change this in Settings.',
                textAlign: TextAlign.center,
                style: TextStyle(color: c.muted, fontSize: 15, height: 1.35),
              ),
              if (note != null) ...[
                const SizedBox(height: 12),
                Text(
                  note!,
                  textAlign: TextAlign.center,
                  style: TextStyle(color: scheme.error),
                ),
              ],
              const SizedBox(height: 24),
              SizedBox(
                height: 52,
                child: WorkingButton(
                  label: 'Turn on',
                  working: working,
                  onPressed: turnOn,
                ),
              ),
              const SizedBox(height: 8),
              SizedBox(
                height: 48,
                child: TextButton(
                  onPressed: working
                      ? null
                      : () => Navigator.of(context).pop(false),
                  child: const Text('Not now'),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
