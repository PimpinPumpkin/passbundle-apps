// Updates from PassBundle's GitHub releases, on Android, the Mac, Windows,
// and Linux (README, "Android test releases", "Mac test releases", and
// "Windows and Linux test releases"); the app's own native code does the
// work (Android: update/Updater.kt; the Mac: macos/Runner/Updater.swift;
// Windows and Linux: the core, core/src/updates). In Settings: this
// version, checking, and updating, with what's new. When the check on start
// (at most once a day) finds one, a quiet notice over the vault. Nothing
// installs without the person's tap, and on Android Android's own prompt;
// on computers PassBundle takes the new version's place and restarts.
// Elsewhere (iOS) there are no updates here.
import 'dart:async';
import 'dart:io';
import 'dart:ui' show AppExitType;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'theme.dart';

/// What the app's native code does for updates (Android: MainActivity.kt;
/// the Mac: MainFlutterWindow.swift; Windows and Linux: the core), as maps.
abstract class UpdatePlatform {
  /// `version`, `store` (the app store that installed PassBundle, or null),
  /// `auto` (whether it checks once a day), `restarts` (whether updating
  /// restarts PassBundle, as on computers), `installNote` (why this copy
  /// can't update itself, so an update is only news), and `failed` (an
  /// update that didn't happen: `{version, reason}`).
  Future<Map<String, dynamic>> status();

  /// `{ok, available, version, notes}` or `{ok: false, error}`; `quiet` is
  /// the check on start, which the native code skips if it checked today.
  Future<Map<String, dynamic>> check({required bool quiet});

  /// Downloads, checks, and hands the update to Android's installer, or on
  /// the Mac puts it in this copy's place: `{ok}` (with `restart` when
  /// PassBundle restarts now) or `{ok: false, error}`, with `reveal` when
  /// the checked update can be shown in Finder for the person to put in
  /// place (PassBundle can't change the folder it's in).
  Future<Map<String, dynamic>> install();

  /// Shows that checked update in Finder.
  Future<bool> reveal();

  Future<void> setAuto(bool on);

  /// How far the download is (`progress`, `{percent}`) and what Android's
  /// installer said, or on the Mac that PassBundle couldn't restart itself
  /// (`installed`, `{ok, error}`).
  set onEvent(void Function(String what, Map<String, dynamic> details) handler);
}

class ChannelUpdatePlatform implements UpdatePlatform {
  static const _platform = MethodChannel('passbundle/platform');
  static const _events = MethodChannel('passbundle/updates');

  Future<Map<String, dynamic>> _call(
    String method, [
    Map<String, dynamic>? arguments,
  ]) async {
    try {
      return Map<String, dynamic>.from(
        await _platform.invokeMethod<Map>(method, arguments) ?? {'ok': false},
      );
    } on PlatformException {
      return {'ok': false};
    }
  }

  @override
  Future<Map<String, dynamic>> status() => _call('updateStatus');

  @override
  Future<Map<String, dynamic>> check({required bool quiet}) =>
      _call('updateCheck', {'quiet': quiet});

  @override
  Future<Map<String, dynamic>> install() => _call('updateInstall');

  @override
  Future<bool> reveal() async {
    try {
      return await _platform.invokeMethod<bool>('updateReveal') ?? false;
    } on PlatformException {
      return false;
    }
  }

  @override
  Future<void> setAuto(bool on) =>
      _platform.invokeMethod<bool>('updateAuto', {'on': on});

  @override
  set onEvent(void Function(String, Map<String, dynamic>) handler) =>
      _events.setMethodCallHandler((call) async {
        handler(
          call.method,
          Map<String, dynamic>.from((call.arguments as Map?) ?? {}),
        );
      });
}

/// Windows and Linux: the core checks, downloads, and checks again (its
/// manifest's signature too), then quits PassBundle for its updater to put
/// the new version in place and open it (core/src/updates).
class CoreUpdatePlatform implements UpdatePlatform {
  CoreUpdatePlatform([Core? core]) : _core = core ?? Core();

  final Core _core;
  void Function(String, Map<String, dynamic>)? _heard;

  @override
  Future<Map<String, dynamic>> status() async => _core.updateStatus();

  @override
  Future<Map<String, dynamic>> check({required bool quiet}) =>
      Core.updateCheck(quiet);

  @override
  Future<Map<String, dynamic>> install() async {
    // The download is on another isolate; how far it is, from here.
    final progress = Timer.periodic(const Duration(milliseconds: 250), (_) {
      final percent = _core.updateProgress()['percent'];
      if (percent is int) _heard?.call('progress', {'percent': percent});
    });
    final Map<String, dynamic> answer;
    try {
      answer = await Core.updateInstall();
    } finally {
      progress.cancel();
    }
    if (answer['ok'] == true && answer['restart'] == true) {
      // Its updater waits for this copy to quit; long enough to say so first.
      Timer(const Duration(milliseconds: 700), () {
        ServicesBinding.instance.exitApplication(AppExitType.required);
      });
    }
    return answer;
  }

  @override
  Future<bool> reveal() async => false;

  @override
  Future<void> setAuto(bool on) async => _core.updateAuto(on);

  @override
  set onEvent(void Function(String, Map<String, dynamic>) handler) =>
      _heard = handler;

  /// The app's first frame is drawn: an update waiting on it to start can
  /// finish (core/src/updates/swap.rs).
  Map<String, dynamic> started() => _core.updateStarted();
}

/// Updates on this device: what Settings and the notice show.
class Updates extends ChangeNotifier {
  Updates(this.platform) {
    platform.onEvent = _heard;
  }

  final UpdatePlatform platform;

  /// This build's version, and the store that keeps it up to date, if any.
  String? version;
  String? store;
  bool auto = true;
  bool loaded = false;

  /// Updating restarts PassBundle (the Mac), and it's restarting now.
  bool restarts = false;
  bool restarting = false;

  /// The update is downloaded and checked, and can be shown in Finder.
  bool revealable = false;

  /// Why this copy can't update itself (a folder only an administrator can
  /// change, say): an update found is news, with this instead of a button.
  String? installNote;

  /// An update that didn't happen (it didn't start, and the old version
  /// came back), in the person's words, for the notice; once.
  String? _failure;

  /// That's what happened as PassBundle opened this time.
  bool cameBack = false;

  /// A newer release: `version` and `notes`.
  Map<String, dynamic>? offer;
  bool checking = false;
  bool upToDate = false;

  /// While the update downloads: how far, 0 to 100.
  int? percent;

  /// What went wrong, or what's happening, in the person's words.
  String? note;

  /// The check on start found an update the notice hasn't shown yet.
  bool _notice = false;

  Future<void> load() async {
    final status = await platform.status();
    version = status['version'] as String?;
    store = status['store'] as String?;
    auto = status['auto'] != false;
    restarts = status['restarts'] == true;
    installNote = status['installNote'] as String?;
    final failed = status['failed'];
    if (failed is Map && !cameBack) {
      cameBack = true;
      _failure = failed['reason'] as String?;
      note = _failure;
    }
    loaded = true;
    notifyListeners();
  }

  Future<void> check({bool quiet = false}) async {
    if (checking || percent != null) return;
    checking = true;
    if (!quiet) note = null;
    notifyListeners();
    final answer = await platform.check(quiet: quiet);
    checking = false;
    if (answer['ok'] == true && answer['skipped'] != true) {
      final available = answer['available'] == true;
      offer = available
          ? {'version': answer['version'], 'notes': answer['notes'] ?? ''}
          : null;
      upToDate = !available;
      _notice = quiet && available;
    } else if (answer['ok'] != true && !quiet) {
      note = answer['error'] as String? ?? "Couldn't check for updates.";
    }
    notifyListeners();
  }

  Future<void> install() async {
    if (offer == null || percent != null || restarting) return;
    percent = 0;
    note = null;
    revealable = false;
    notifyListeners();
    final answer = await platform.install();
    percent = null;
    final ok = answer['ok'] == true;
    restarting = ok && answer['restart'] == true;
    revealable = !ok && answer['reveal'] == true;
    note = ok
        ? null
        : answer['error'] as String? ?? "Couldn't update PassBundle.";
    notifyListeners();
  }

  /// Shows the checked update in Finder, for the person to put in place.
  Future<void> reveal() async {
    if (!revealable) return;
    await platform.reveal();
  }

  Future<void> setAuto(bool on) async {
    auto = on;
    notifyListeners();
    await platform.setAuto(on);
  }

  /// The update the check on start found, once, for the notice; else null.
  String? takeNotice() {
    if (!_notice) return null;
    _notice = false;
    return offer?['version'] as String?;
  }

  /// An update that didn't happen, once, for the notice; else null.
  String? takeFailure() {
    final failure = _failure;
    _failure = null;
    return failure;
  }

  void _heard(String what, Map<String, dynamic> details) {
    if (what == 'progress' && percent != null) {
      percent = details['percent'] as int?;
    } else if (what == 'installed' && details['ok'] != true) {
      restarting = false;
      note = details['error'] as String? ?? "PassBundle wasn't updated.";
    }
    notifyListeners();
  }
}

/// Not tests: flutter test runs on computers too, and tests put one with a
/// stand-in platform here. A product build never takes FLUTTER_TEST from its
/// environment.
final _notTesting =
    const bool.fromEnvironment('dart.vm.product') ||
    !Platform.environment.containsKey('FLUTTER_TEST');

/// Updates on this device: Android, the Mac, Windows, and Linux, where
/// PassBundle updates itself.
Updates? updates = Platform.isAndroid
    ? Updates(ChannelUpdatePlatform())
    : Platform.isMacOS && _notTesting
    ? Updates(ChannelUpdatePlatform())
    : (Platform.isWindows || Platform.isLinux) && _notTesting
    ? Updates(CoreUpdatePlatform())
    : null;

/// The once-a-day check, as the app starts; quiet, whatever it finds. In a
/// build with test hooks (never a release), PB_UPDATE_NOW=1 checks and
/// updates at once, for tools/desktop-update-test.sh; not after an update
/// that didn't happen, which would only try it again.
Future<void> checkForUpdatesOnStart() async {
  final u = updates;
  if (u == null) return;
  await u.load();
  if (testSetting('PB_UPDATE_NOW') == '1' && !u.cameBack) {
    await u.check();
    if (u.offer != null) await u.install();
    return;
  }
  if (u.store == null) await u.check(quiet: true);
}

/// The first frame is drawn: on Windows and Linux, an update that just put
/// this version in place hears it started, and its leftovers go.
void updateStarted() {
  final platform = updates?.platform;
  if (platform is CoreUpdatePlatform) platform.started();
}

/// The notice over the vault when the check on start found an update: once,
/// and it only offers to show it. Or, once, that an update didn't happen.
void showUpdateNotice(BuildContext context) {
  final failure = updates?.takeFailure();
  if (failure != null) {
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(failure), duration: const Duration(seconds: 10)),
    );
    return;
  }
  final version = updates?.takeNotice();
  if (version == null) return;
  ScaffoldMessenger.of(context).showSnackBar(
    SnackBar(
      content: Text('PassBundle $version is out.'),
      duration: const Duration(seconds: 8),
      action: SnackBarAction(
        label: 'See update',
        onPressed: () => showDialog<void>(
          context: context,
          builder: (_) => const UpdateDialog(),
        ),
      ),
    ),
  );
}

/// The update on its own, from the notice: what's new, and updating.
class UpdateDialog extends StatelessWidget {
  const UpdateDialog({super.key});

  @override
  Widget build(BuildContext context) {
    final u = updates!;
    return AlertDialog(
      title: const Text('Update PassBundle'),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(child: UpdateOffer(updates: u)),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Not now'),
        ),
      ],
    );
  }
}

/// Settings' Updates: this version, checking, updating, and the daily check.
class UpdatesSection extends StatefulWidget {
  const UpdatesSection({super.key, required this.updates});

  final Updates updates;

  @override
  State<UpdatesSection> createState() => _UpdatesSectionState();
}

class _UpdatesSectionState extends State<UpdatesSection> {
  @override
  void initState() {
    super.initState();
    if (!widget.updates.loaded) widget.updates.load();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.updates,
    builder: (context, _) {
      final u = widget.updates;
      final store = u.store;
      final String? said = store != null
          ? 'Updates come from $store.'
          : u.offer != null
          ? 'PassBundle ${u.offer!['version']} is out.'
          : u.upToDate
          ? 'This is the newest version.'
          : null;
      final primary = Theme.of(context).colorScheme.primary;
      // A group like Settings' others; checking is a row of its own, so
      // nothing squeezes the version beside it.
      return SettingsGroup(
        title: 'Updates',
        footer: store != null
            ? null
            : 'The daily check is as PassBundle opens. Checking asks GitHub, '
                  'where PassBundle is published, and nothing else.',
        children: [
          ListTile(
            title: Text(
              u.version == null ? 'PassBundle' : 'Version ${u.version}',
            ),
            subtitle: said == null ? null : Text(said),
          ),
          if (store == null) ...[
            ListTile(
              title: Text(
                'Check for updates',
                style: TextStyle(
                  color: u.loaded ? primary : context.colors.muted,
                  fontWeight: FontWeight.w600,
                ),
              ),
              trailing: u.checking
                  ? const SizedBox(
                      width: 20,
                      height: 20,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : null,
              onTap: u.loaded && !u.checking ? () => u.check() : null,
            ),
            // What went wrong shows by what the person tapped.
            if (u.offer != null)
              Padding(
                padding: const EdgeInsets.all(16),
                child: UpdateOffer(updates: u),
              )
            else if (u.note != null)
              Padding(
                padding: const EdgeInsets.all(16),
                child: Text(
                  u.note!,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
              ),
            SwitchListTile(
              value: u.auto,
              onChanged: u.loaded ? (on) => u.setAuto(on) : null,
              title: const Text('Check once a day'),
              subtitle: const Text('It never updates without you'),
            ),
          ],
        ],
      );
    },
  );
}

/// An update on offer: what's new, and "Update to ...", or how far the
/// download is.
class UpdateOffer extends StatelessWidget {
  const UpdateOffer({super.key, required this.updates});

  final Updates updates;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: updates,
    builder: (context, _) {
      final offer = updates.offer;
      if (offer == null) return const SizedBox.shrink();
      final notes = (offer['notes'] as String? ?? '').trim();
      final percent = updates.percent;
      final muted = TextStyle(color: context.colors.muted, fontSize: 13);
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          if (notes.isNotEmpty) ...[
            Text(
              "What's new in ${offer['version']}",
              style: const TextStyle(fontWeight: FontWeight.w600),
            ),
            const SizedBox(height: 4),
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 200),
              child: SingleChildScrollView(child: Text(notes, style: muted)),
            ),
            const SizedBox(height: 12),
          ],
          if (updates.restarting) ...[
            const LinearProgressIndicator(),
            const SizedBox(height: 6),
            Text('Restarting PassBundle', style: muted),
          ] else if (updates.installNote case final installNote?) ...[
            // It can't replace itself: the news, and how to get it.
            Text(installNote, style: muted),
          ] else if (percent == null) ...[
            FilledButton(
              onPressed: updates.install,
              child: Text('Update to ${offer['version']}'),
            ),
            if (updates.restarts) ...[
              const SizedBox(height: 6),
              Text('PassBundle restarts to finish.', style: muted),
            ],
          ] else ...[
            LinearProgressIndicator(value: percent == 0 ? null : percent / 100),
            const SizedBox(height: 6),
            Text(
              percent >= 100 ? 'Checking the download' : 'Downloading',
              style: muted,
            ),
          ],
          if (updates.note != null) ...[
            const SizedBox(height: 8),
            Text(
              updates.note!,
              style: TextStyle(color: Theme.of(context).colorScheme.error),
            ),
          ],
          if (updates.revealable)
            Align(
              alignment: Alignment.centerLeft,
              child: TextButton.icon(
                onPressed: updates.reveal,
                icon: const Icon(Icons.folder_open_outlined),
                label: const Text('Show in Finder'),
              ),
            ),
          const SizedBox(height: 8),
        ],
      );
    },
  );
}
