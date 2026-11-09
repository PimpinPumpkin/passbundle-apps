// Turning on PassBundle's autofill on Android in one tap, rather than left for
// people to find in Android's settings (docs/phones.md, "Turning it on"), and
// its AutoFill on an iPhone, where "Turn on" opens Settings' own page for it
// (iOS 17 and later) and the person switches PassBundle on there.
// When the vault opens and PassBundle isn't Android's autofill service yet,
// a sheet offers it: "Turn on" opens Android's own request to use PassBundle
// (AutofillSetup.kt), where the person chooses it and confirms, and on
// Android 14 and later a second row opens the system's list of passkey
// providers, where the person switches PassBundle on. Each row says On, with a check, once Android says so: the app asks
// again each time it's back in front. "Later", or closing the sheet, isn't
// asked again; the vault says Settings has the same card, a tap away.
// Nothing turns on without the person's yes in Android's own screens, and
// unlocking is as it was.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';

/// What Android says about PassBundle here (AutofillSetup.kt), or iOS
/// (AppDelegate.swift, AutoFillSetup).
@immutable
class AutofillStatus {
  const AutofillStatus({
    required this.apiLevel,
    this.ios = false,
    this.autofillSupported = true,
    this.autofillOn = false,
    this.passkeysAvailable = false,
    this.passkeysOn = false,
  });

  factory AutofillStatus.from(Map<String, dynamic> phone) => AutofillStatus(
    apiLevel: phone['apiLevel'] as int? ?? 0,
    ios: phone['platform'] == 'ios',
    autofillSupported: phone['autofillSupported'] == true,
    autofillOn: phone['autofillOn'] == true,
    passkeysAvailable: phone['passkeysAvailable'] == true,
    passkeysOn: phone['passkeysOn'] == true,
  );

  /// An iPhone: one switch in Settings for AutoFill, and Apple's words.
  final bool ios;

  /// Android's API level: 29 is Android 10, 34 is Android 14.
  final int apiLevel;

  /// Android has autofill for this person (not every phone or work profile does).
  final bool autofillSupported;

  /// PassBundle is Android's autofill service.
  final bool autofillOn;

  /// Android has Credential Manager, which takes passkey providers.
  final bool passkeysAvailable;

  /// PassBundle's passkey provider is switched on.
  final bool passkeysOn;
}

/// A row of the card.
enum SetupRow { autofill, passkeys }

/// Android 14: the first that takes passkeys from a provider other than
/// Google's.
const passkeysSinceApi = 34;

/// The card's rows, each with whether it's on: autofill where Android has
/// it, and passkeys on Android 14 and later where Credential Manager is. An
/// iPhone has the one: AutoFill.
List<(SetupRow, bool)> setupRows(AutofillStatus status) => [
  if (status.autofillSupported) (SetupRow.autofill, status.autofillOn),
  if (!status.ios &&
      status.apiLevel >= passkeysSinceApi &&
      status.passkeysAvailable)
    (SetupRow.passkeys, status.passkeysOn),
];

/// Whether the vault offers the step as it opens: Android has autofill,
/// PassBundle isn't its service yet, and the person hasn't said "Later".
bool offersAutofill(AutofillStatus? status, {required bool putOff}) =>
    status != null &&
    status.autofillSupported &&
    !status.autofillOn &&
    !putOff;

/// Android's answer, or iOS's; null on a computer.
Future<AutofillStatus?> readAutofillStatus() async {
  final phone = await Core.autofillStatus();
  return phone == null ? null : AutofillStatus.from(phone);
}

/// Where "Later" is remembered: beside the vault, in the app's own folder.
File? get _putOff {
  final dir = Core.folder;
  return dir == null ? null : File('$dir/autofill-offer.json');
}

/// Whether the person said "Later" here.
bool autofillPutOff() => _putOff?.existsSync() ?? false;

/// "Later": the vault doesn't offer it again, and Settings has it.
void putOffAutofill() {
  try {
    _putOff?.writeAsStringSync(
      jsonEncode({'laterAt': DateTime.now().toUtc().toIso8601String()}),
    );
  } on FileSystemException {
    // Then it's offered again next time the vault opens.
  }
}

/// What the vault says after "Later": where it is from now on (with a way
/// to Settings). An iPhone says AutoFill, as iOS does.
String get autofillLater =>
    'You can turn on ${isIPhone ? 'AutoFill' : 'autofill'} in Settings any time.';

/// As the vault opens on Android or an iPhone: the step, if
/// [offersAutofill]. What to say once it closes ("Autofill is on.", or
/// [autofillLater]), or null.
Future<String?> offerAutofill(BuildContext context) async {
  if (!(isAndroid || isIPhone) || autofillPutOff()) return null;
  final status = await readAutofillStatus();
  if (!context.mounted || !offersAutofill(status, putOff: false)) return null;
  final after = await showModalBottomSheet<AutofillStatus>(
    context: context,
    isScrollControlled: true,
    builder: (_) => AutofillOffer(status: status!),
  );
  if (after == null || !after.autofillOn) {
    putOffAutofill();
    return autofillLater;
  }
  if (after.ios) return 'AutoFill is on.';
  final passkeys = setupRows(after).contains((SetupRow.passkeys, true));
  return passkeys ? 'Autofill and passkeys are on.' : 'Autofill is on.';
}

/// The card: a row for autofill and, on Android 14 and later, one for
/// passkeys, each On with a check or a "Turn on" button. In the step after
/// unlocking, and in Settings under [heading]. Nothing where Android has
/// neither.
class AutofillCard extends StatefulWidget {
  const AutofillCard({super.key, this.status, this.heading, this.onStatus});

  /// Known already (the step's), else asked for.
  final AutofillStatus? status;

  /// Settings' heading over the card; the step has a title of its own.
  final String? heading;

  /// Each new answer from Android.
  final ValueChanged<AutofillStatus>? onStatus;

  @override
  State<AutofillCard> createState() => _AutofillCardState();
}

class _AutofillCardState extends State<AutofillCard> {
  late AutofillStatus? status = widget.status;
  String? note;

  /// A "Turn on" was tapped: on coming back, Android is asked again a moment
  /// later too, since its own screens can take a moment to tell autofill.
  bool asked = false;
  Timer? again;

  /// Back from Android's prompt or settings (or anywhere else): asked again.
  late final AppLifecycleListener lifecycle = AppLifecycleListener(
    onResume: () {
      refresh();
      if (asked) {
        again?.cancel();
        again = Timer(const Duration(seconds: 1), refresh);
      }
    },
  );

  @override
  void initState() {
    super.initState();
    lifecycle;
    if (status == null) refresh();
  }

  @override
  void dispose() {
    again?.cancel();
    lifecycle.dispose();
    super.dispose();
  }

  Future<void> refresh() async {
    final next = await readAutofillStatus();
    if (!mounted || next == null) return;
    // Just turned on in iOS's Settings: iOS takes the logins to suggest now,
    // rather than at the next sync.
    if (next.ios && next.autofillOn && status?.autofillOn != true) {
      Core.suggest();
    }
    setState(() => status = next);
    widget.onStatus?.call(next);
  }

  Future<void> turnOn(SetupRow row) async {
    setState(() => note = null);
    asked = true;
    final opened = switch (row) {
      SetupRow.autofill => await Core.turnOnAutofill() != null,
      SetupRow.passkeys => await Core.openPasskeySettings(),
    };
    if (!mounted || opened) return;
    setState(() {
      note = switch (row) {
        SetupRow.autofill when status?.ios == true =>
          "iOS didn't open its AutoFill settings. In Settings, open General, then AutoFill & Passwords, and turn on PassBundle.",
        SetupRow.autofill =>
          "Android didn't open its autofill settings. In Settings, search for Autofill service and choose PassBundle.",
        SetupRow.passkeys =>
          "Android didn't open its passkey settings. In Settings, search for Passwords and turn on PassBundle.",
      };
    });
  }

  Widget tile(SetupRow row, bool on) {
    final (title, subtitle) = switch (row) {
      SetupRow.autofill when status?.ios == true => (
        'Use PassBundle for AutoFill',
        'Fill logins and sign in with passkeys in apps and Safari',
      ),
      SetupRow.autofill => (
        'Use PassBundle for autofill',
        'Fill logins in apps and browsers',
      ),
      SetupRow.passkeys => (
        'Use PassBundle for passkeys',
        'Sign in with passkeys in apps and browsers',
      ),
    };
    final primary = Theme.of(context).colorScheme.primary;
    return ListTile(
      key: ValueKey('setup-${row.name}'),
      title: Text(title),
      subtitle: Text(subtitle),
      trailing: on
          ? Semantics(
              label: 'On',
              excludeSemantics: true,
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Icon(Icons.check_circle_rounded, color: primary, size: 20),
                  const SizedBox(width: 6),
                  Text(
                    'On',
                    style: TextStyle(
                      color: primary,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ],
              ),
            )
          : FilledButton(
              key: ValueKey('turn-on-${row.name}'),
              onPressed: () => turnOn(row),
              child: const Text('Turn on'),
            ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final status = this.status;
    final rows = status == null ? const <(SetupRow, bool)>[] : setupRows(status);
    if (rows.isEmpty) return const SizedBox.shrink();
    final tiles = [for (final (row, on) in rows) tile(row, on)];
    final error = Theme.of(context).colorScheme.error;
    final heading = widget.heading;
    if (heading != null) {
      return SettingsGroup(
        title: heading,
        footer: note,
        footerColor: error,
        children: tiles,
      );
    }
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        RestingCard(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (var i = 0; i < tiles.length; i++) ...[
                if (i > 0) const Divider(indent: 16),
                tiles[i],
              ],
            ],
          ),
        ),
        if (note != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 8, 4, 0),
            child: Text(
              note!,
              style: TextStyle(color: error, fontSize: 13, height: 1.35),
            ),
          ),
      ],
    );
  }
}

/// The step: what it is, the card, and "Later" ("Done" once autofill is
/// on). It closes by itself once every row is on. It answers with Android's
/// last word.
class AutofillOffer extends StatefulWidget {
  const AutofillOffer({super.key, required this.status});

  final AutofillStatus status;

  @override
  State<AutofillOffer> createState() => _AutofillOfferState();
}

class _AutofillOfferState extends State<AutofillOffer> {
  late AutofillStatus status = widget.status;
  bool closing = false;

  void changed(AutofillStatus next) {
    if (closing) return;
    setState(() => status = next);
    final rows = setupRows(next);
    if (rows.isNotEmpty && rows.every((row) => row.$2)) {
      closing = true;
      Navigator.of(context).pop(next);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final scheme = Theme.of(context).colorScheme;
    return SafeArea(
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
                  Icons.password_rounded,
                  size: 34,
                  color: scheme.primary,
                ),
              ),
            ),
            const SizedBox(height: 16),
            Text(
              status.ios ? 'Turn on AutoFill' : 'Turn on autofill',
              textAlign: TextAlign.center,
              style: const TextStyle(fontSize: 22, fontWeight: FontWeight.w700),
            ),
            const SizedBox(height: 8),
            Text(
              status.ios
                  ? 'Settings opens where you choose what fills your '
                        'passwords: turn on PassBundle. You can also do this '
                        'later in Settings.'
                  : 'Android asks which service fills your passwords: choose '
                        'PassBundle. You can also do this later in Settings.',
              textAlign: TextAlign.center,
              style: TextStyle(color: c.muted, fontSize: 15, height: 1.35),
            ),
            const SizedBox(height: 20),
            AutofillCard(status: status, onStatus: changed),
            const SizedBox(height: 12),
            SizedBox(
              height: 48,
              child: TextButton(
                onPressed: closing
                    ? null
                    : () {
                        closing = true;
                        Navigator.of(context).pop(status);
                      },
                child: Text(status.autofillOn ? 'Done' : 'Later'),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
