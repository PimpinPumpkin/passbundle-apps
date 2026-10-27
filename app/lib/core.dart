// The core (Rust), through its C bridge (core/src/ffi.rs). No key crosses
// here, and a secret only when the person reveals it or edits it
// (docs/core.md, "Memory"). Slow calls (signing in, unlocking, syncing,
// saving) run on a background isolate; the screens don't read while one runs.
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:isolate';

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart' show visibleForTesting;
import 'package:flutter/services.dart';

typedef _Str0 = Pointer<Utf8> Function();
typedef _Str1 = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _Str2 = Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>);
typedef _Str3 = Pointer<Utf8> Function(
  Pointer<Utf8>,
  Pointer<Utf8>,
  Pointer<Utf8>,
);
typedef _Str5 = Pointer<Utf8> Function(
  Pointer<Utf8>,
  Pointer<Utf8>,
  Pointer<Utf8>,
  Pointer<Utf8>,
  Pointer<Utf8>,
);
typedef _FlagNative = Pointer<Utf8> Function(Bool);
typedef _Flag = Pointer<Utf8> Function(bool);
typedef _FinishNative = Pointer<Utf8> Function(Uint8, Pointer<Utf8>, Bool);
typedef _Finish = Pointer<Utf8> Function(int, Pointer<Utf8>, bool);

/// A product build: `flutter build` in release mode, as the releases are.
const product = bool.fromEnvironment('dart.vm.product');

/// Whether this build takes test settings from its environment (PB_MEASURE,
/// PB_MEASURE_PASSWORD, PB_MEASURE_FILE, PB_THEME): tests and debug builds,
/// and the release-mode builds CI makes for the measuring tools
/// (`--dart-define=PB_TEST_HOOKS=true`, with the core's test-hooks feature).
/// The releases are built without, so they ignore all of these.
const testHooks = !product || bool.fromEnvironment('PB_TEST_HOOKS');

/// A test setting from the environment, or null where [testHooks] is off.
String? testSetting(String name) =>
    testHooks ? Platform.environment[name] : null;

DynamicLibrary _open() {
  // Tests and tools point at a library they built. Never in a product
  // build: whatever library this opens is the core, and gets the master
  // password and the keychain's trust.
  if (!product) {
    final override = Platform.environment['PB_CORE_LIB'];
    if (override != null) return DynamicLibrary.open(override);
  }
  final dir = File(Platform.resolvedExecutable).parent.path;
  if (Platform.isLinux) {
    return DynamicLibrary.open('$dir/lib/libpassbundle_core.so');
  }
  if (Platform.isWindows) {
    return DynamicLibrary.open('$dir\\passbundle_core.dll');
  }
  if (Platform.isMacOS) {
    return DynamicLibrary.open('$dir/../Frameworks/libpassbundle_core.dylib');
  }
  if (Platform.isAndroid) return DynamicLibrary.open('libpassbundle_core.so');
  return DynamicLibrary.process();
}

/// A phone: biometric unlock is the phone's own, with no desktop agent or
/// browser extension sharing it.
bool get isPhone => debugPhone ?? (Platform.isAndroid || Platform.isIOS);

/// Tests only: the phone's screens and words on a computer, with a stand-in
/// for the phone's own code (its method channel). Null is what this is.
@visibleForTesting
bool? debugPhone;

/// An answer from a call that can fail: ok, or the core's words.
class Answer {
  Answer(Map<String, dynamic> json)
    : ok = json['ok'] == true,
      error = json['error'] as String?,
      signedOut = json['signedOut'] == true,
      body = json;

  final bool ok;
  final String? error;
  final bool signedOut;
  final Map<String, dynamic> body;
}

class Core {
  Core() : _lib = _open();

  final DynamicLibrary _lib;

  late final _openAt = _lib
      .lookupFunction<
        Pointer<Utf8> Function(Pointer<Utf8>, Bool),
        Pointer<Utf8> Function(Pointer<Utf8>, bool)
      >('pb_open');
  late final _version = _lib.lookupFunction<_Str0, _Str0>('pb_version');
  late final _useAgentHome = _lib.lookupFunction<_Str2, _Str2>(
    'pb_use_agent_home',
  );
  late final _startAgent = _lib.lookupFunction<_Str0, _Str0>('pb_start_agent');
  late final _bioStatus = _lib.lookupFunction<_Str0, _Str0>(
    'pb_biometric_status',
  );
  late final _bioEnroll = _lib.lookupFunction<_Str0, _Str0>(
    'pb_biometric_enroll',
  );
  late final _bioUnlock = _lib.lookupFunction<_Str0, _Str0>(
    'pb_biometric_unlock',
  );
  late final _bioRemove = _lib.lookupFunction<_Str0, _Str0>(
    'pb_biometric_remove',
  );
  late final _state = _lib.lookupFunction<_Str0, _Str0>('pb_state');
  late final _servers = _lib.lookupFunction<_Str0, _Str0>('pb_servers');
  late final _signIn = _lib.lookupFunction<_Str3, _Str3>('pb_sign_in');
  late final _signUp = _lib.lookupFunction<_Str5, _Str5>('pb_sign_up');
  late final _finish = _lib.lookupFunction<_FinishNative, _Finish>(
    'pb_finish_sign_in',
  );
  late final _sendEmail = _lib.lookupFunction<_Str0, _Str0>(
    'pb_send_email_code',
  );
  late final _unlock = _lib.lookupFunction<_Str1, _Str1>('pb_unlock');
  late final _saveAttachment = _lib.lookupFunction<_Str2, _Str2>(
    'pb_save_attachment',
  );
  late final _confirmMasterPassword = _lib.lookupFunction<_Str2, _Str2>(
    'pb_confirm_master_password',
  );
  late final _signInRequests = _lib.lookupFunction<_Str0, _Str0>(
    'pb_sign_in_requests',
  );
  late final _answerSignInRequest = _lib.lookupFunction<_Str3, _Str3>(
    'pb_answer_sign_in_request',
  );
  late final _confirmWithMasterPassword = _lib.lookupFunction<_Str1, _Str1>(
    'pb_confirm_with_master_password',
  );
  late final _approvals = _lib.lookupFunction<_Str0, _Str0>('pb_approvals');
  late final _setApprovals = _lib.lookupFunction<_Str1, _Str1>(
    'pb_set_approvals',
  );
  late final _sync = _lib.lookupFunction<_Str0, _Str0>('pb_sync');
  late final _syncSettings = _lib.lookupFunction<_Str0, _Str0>(
    'pb_sync_settings',
  );
  late final _lock = _lib.lookupFunction<_Str0, _Str0>('pb_lock');
  late final _signOut = _lib.lookupFunction<_Str0, _Str0>('pb_sign_out');
  late final _preferences = _lib.lookupFunction<_Str0, _Str0>('pb_preferences');
  late final _lockPolicy = _lib.lookupFunction<_Str0, _Str0>('pb_lock_policy');
  late final _setPreferences = _lib.lookupFunction<_Str1, _Str1>(
    'pb_set_preferences',
  );
  late final _used = _lib.lookupFunction<_Str0, _Str0>('pb_used');
  late final _tick = _lib.lookupFunction<_Str0, _Str0>('pb_tick');
  late final _sleeping = _lib.lookupFunction<_Str0, _Str0>('pb_sleeping');
  late final _left = _lib.lookupFunction<_Str0, _Str0>('pb_left');
  late final _places = _lib.lookupFunction<_Str0, _Str0>('pb_places');
  late final _templates = _lib.lookupFunction<_Str0, _Str0>('pb_templates');
  late final _icon = _lib.lookupFunction<_Str1, _Str1>('pb_icon');
  late final _testIcons = _lib.lookupFunction<_Str1, _Str1>('pb_test_icons');
  late final _keepGenerated = _lib.lookupFunction<_Str1, _Str1>(
    'pb_keep_generated',
  );
  late final _appearance = _lib.lookupFunction<_Str0, _Str0>('pb_appearance');
  late final _setAppearance = _lib.lookupFunction<_Str1, _Str1>(
    'pb_set_appearance',
  );
  late final _health = _lib.lookupFunction<_Str0, _Str0>('pb_health');
  late final _history = _lib.lookupFunction<_Str1, _Str1>('pb_history');
  late final _itemVersion = _lib.lookupFunction<_Str3, _Str3>(
    'pb_item_version',
  );
  late final _restoreVersion = _lib.lookupFunction<_Str2, _Str2>(
    'pb_restore_version',
  );
  late final _deleteHistory = _lib.lookupFunction<_Str2, _Str2>(
    'pb_delete_history',
  );
  late final _setFavorite = _lib.lookupFunction<_Str2, _Str2>(
    'pb_set_favorite',
  );
  late final _list = _lib.lookupFunction<_Str1, _Str1>('pb_list');
  late final _item = _lib.lookupFunction<_Str1, _Str1>('pb_item');
  late final _reveal = _lib.lookupFunction<_Str2, _Str2>('pb_reveal');
  late final _copy = _lib.lookupFunction<_Str2, _Str2>('pb_copy');
  late final _totp = _lib.lookupFunction<_Str1, _Str1>('pb_totp');
  late final _save = _lib.lookupFunction<_Str2, _Str2>('pb_save');
  late final _trash = _lib.lookupFunction<_Str1, _Str1>('pb_trash');
  late final _createFolder = _lib.lookupFunction<_Str1, _Str1>(
    'pb_create_folder',
  );
  late final _renameFolder = _lib.lookupFunction<_Str2, _Str2>(
    'pb_rename_folder',
  );
  late final _deleteFolder = _lib.lookupFunction<_Str1, _Str1>(
    'pb_delete_folder',
  );
  late final _generate = _lib.lookupFunction<_Str1, _Str1>('pb_generate');
  late final _generatorOptions = _lib.lookupFunction<_Str0, _Str0>(
    'pb_generator_options',
  );
  late final _generatorPolicy = _lib.lookupFunction<_Str0, _Str0>(
    'pb_generator_policy',
  );
  late final _updateStatus = _lib.lookupFunction<_Str0, _Str0>(
    'pb_update_status',
  );
  late final _updateCheck = _lib.lookupFunction<_FlagNative, _Flag>(
    'pb_update_check',
  );
  late final _updateInstall = _lib.lookupFunction<_Str0, _Str0>(
    'pb_update_install',
  );
  late final _updateProgress = _lib.lookupFunction<_Str0, _Str0>(
    'pb_update_progress',
  );
  late final _updateAuto = _lib.lookupFunction<_FlagNative, _Flag>(
    'pb_update_auto',
  );
  late final _updateStarted = _lib.lookupFunction<_Str0, _Str0>(
    'pb_update_started',
  );
  late final _free = _lib
      .lookupFunction<
        Void Function(Pointer<Utf8>),
        void Function(Pointer<Utf8>)
      >('pb_free');

  dynamic _answer(Pointer<Utf8> result) {
    if (result == nullptr) return null;
    try {
      return jsonDecode(result.toDartString());
    } finally {
      _free(result);
    }
  }

  T _with<T>(List<String> args, T Function(List<Pointer<Utf8>>) call) {
    final pointers = [for (final a in args) a.toNativeUtf8()];
    try {
      return call(pointers);
    } finally {
      for (final p in pointers) {
        // Wipe before freeing: passwords pass through here.
        final bytes = p.cast<Uint8>().asTypedList(p.length);
        bytes.fillRange(0, bytes.length, 0);
        malloc.free(p);
      }
    }
  }

  /// Runs a slow call on a background isolate (the core is the same library there).
  static Future<Answer> _background(dynamic Function(Core core) call) async =>
      Answer(
        Map<String, dynamic>.from(await Isolate.run(() => call(Core()) as Map)),
      );

  String version() => _answer(_version()) as String;

  // Updates on Windows and Linux (core/src/updates; updates.dart): the
  // checking and downloading on a background isolate.
  Map<String, dynamic> _map(Pointer<Utf8> result) =>
      Map<String, dynamic>.from(_answer(result) as Map? ?? {'ok': false});

  Map<String, dynamic> updateStatus() => _map(_updateStatus());

  Map<String, dynamic> updateProgress() => _map(_updateProgress());

  Map<String, dynamic> updateStarted() => _map(_updateStarted());

  void updateAuto(bool on) => _map(_updateAuto(on));

  static Future<Map<String, dynamic>> updateCheck(bool quiet) =>
      Isolate.run(() {
        final core = Core();
        return core._map(core._updateCheck(quiet));
      });

  static Future<Map<String, dynamic>> updateInstall() => Isolate.run(() {
    final core = Core();
    return core._map(core._updateInstall());
  });

  /// The app on a folder of its own: Android's app folder, or a test's, with
  /// the refresh token in memory for tests and tools.
  Map<String, dynamic> openAt(String dir, {bool memorySecrets = true}) {
    folder = dir;
    return _with([dir], (p) => _answer(_openAt(p[0], memorySecrets)))
        as Map<String, dynamic>;
  }

  /// The folder the vault is kept in, once opened on one: where the app keeps
  /// the little it remembers itself (unlock_offer.dart).
  static String? folder;

  /// What only the phone's own code does (docs/phones.md, "Talking to the OS").
  static const platform = MethodChannel('passbundle/platform');

  /// Phones keep the vault in the app's own folder, which only the OS knows.
  static Future<void> openOnThisDevice() async {
    if (!isPhone) return;
    final dir = await platform.invokeMethod<String>('filesDir');
    Core().openAt(dir!, memorySecrets: false);
    final managed = await platform.invokeMethod<String>('managedServer');
    _managedOnPhone = managed?.trim().replaceAll(RegExp(r'/+$'), '');
  }

  /// Where to sign in: `ours`, PassBundle's own server, and `managed`, the
  /// server an organization set by policy, if any (then it's the one).
  Map<String, dynamic> servers() {
    final servers = _answer(_servers()) as Map<String, dynamic>;
    final phone = _managedOnPhone;
    if (phone != null) servers['managed'] = phone;
    return servers;
  }

  /// On Android, the server an organization's MDM set (its managed
  /// configuration), which only the app's Kotlin can read; read at start.
  static String? _managedOnPhone;

  /// "signedOut", "locked", or "unlocked", with "server" and "email".
  Map<String, dynamic> state() => _answer(_state()) as Map<String, dynamic>;

  static Future<Answer> signIn(String server, String email, String password) =>
      _background(
        (c) => c._with([
          server,
          email,
          password,
        ], (p) => c._answer(c._signIn(p[0], p[1], p[2]))),
      );

  /// Makes an account on the server, then signs in to it (docs/core.md,
  /// "Making an account"). [name] and [hint] may be empty.
  static Future<Answer> signUp(
    String server,
    String email,
    String name,
    String password,
    String hint,
  ) => _background(
    (c) => c._with([
      server,
      email,
      name,
      password,
      hint,
    ], (p) => c._answer(c._signUp(p[0], p[1], p[2], p[3], p[4]))),
  );

  static Future<Answer> finishSignIn(
    int provider,
    String code,
    bool remember,
  ) => _background(
    (c) =>
        c._with([code], (p) => c._answer(c._finish(provider, p[0], remember))),
  );

  static Future<Answer> sendEmailCode() =>
      _background((c) => c._answer(c._sendEmail()));

  static Future<Answer> unlock(String password) => _background(
    (c) => c._with([password], (p) => c._answer(c._unlock(p[0]))),
  );

  /// Downloads one of an item's attachments, opens it, and saves it to the
  /// Downloads folder (docs/core.md, "Attachments"): `{ok, path}`. On a
  /// phone, the phone's own Downloads (Android) or Files (iPhone), through
  /// its native piece, where the core writes the file itself.
  static Future<Answer> saveAttachment(
    String id,
    String attachmentId,
    String name,
  ) async => isPhone
      ? Answer(
          await _onPhone('saveAttachment', {
            'id': id,
            'attachmentId': attachmentId,
            'name': name,
          }),
        )
      : _background(
          (c) => c._with([
            id,
            attachmentId,
          ], (p) => c._answer(c._saveAttachment(p[0], p[1]))),
        );

  /// The master password, for an item that asks for it again (docs/core.md,
  /// "Asking for the master password again"): then its secrets may be used.
  static Future<Answer> confirmMasterPassword(String id, String password) =>
      _background(
        (c) => c._with([
          id,
          password,
        ], (p) => c._answer(c._confirmMasterPassword(p[0], p[1]))),
      );

  static Future<Answer> sync() => _background((c) => c._answer(c._sync()));

  /// Sign-ins waiting for this account's answer (docs/core.md, "Approving a
  /// sign-in"), from the server: `requests`, each `{id, device, ip,
  /// fingerprint, ageSeconds, secondsLeft}`, newest first.
  static Future<Answer> signInRequests() =>
      _background((c) => c._answer(c._signInRequests()));

  /// Approves or denies one. Approving takes the phrase the person was
  /// shown, and may answer `confirmFirst`: confirm it's them, then again.
  static Future<Answer> answerSignInRequest(
    String id,
    String fingerprint,
    bool approve,
  ) => _background(
    (c) => c._with([
      id,
      fingerprint,
      approve ? 'true' : 'false',
    ], (p) => c._answer(c._answerSignInRequest(p[0], p[1], p[2]))),
  );

  /// The master password, as the person confirming it's them to approve a
  /// sign-in.
  static Future<Answer> confirmWithMasterPassword(String password) =>
      _background(
        (c) => c._with([
          password,
        ], (p) => c._answer(c._confirmWithMasterPassword(p[0]))),
      );

  /// Whether approving a sign-in asks for biometrics or the master password
  /// first, on this device.
  bool confirmApprovals() =>
      (_answer(_approvals()) as Map?)?['confirm'] != false;

  Answer setConfirmApprovals(bool on) => Answer(
    _with([on ? 'true' : 'false'], (p) => _answer(_setApprovals(p[0])))
        as Map<String, dynamic>,
  );

  /// Saves the generator's settings to the server, so they're the same on
  /// the person's other devices (core/src/synced_settings.rs).
  static Future<Answer> syncSettings() =>
      _background((c) => c._answer(c._syncSettings()));

  static Future<Answer> save(String? id, Map<String, dynamic> draft) =>
      _background(
        (c) => c._with([
          id ?? '',
          jsonEncode(draft),
        ], (p) => c._answer(c._save(p[0], p[1]))),
      );

  /// Unlocking with Windows Hello or Touch ID, through the desktop agent, or
  /// on a phone with its own biometrics, through the app's Kotlin or Swift:
  /// `status` is setUp, notSetUp, unavailable, or noAgent, and `method` its name.
  static Future<Map<String, dynamic>> biometricStatus() async {
    if (isPhone) return _onPhone('biometricStatus');
    return Map<String, dynamic>.from(
      await Isolate.run(() {
        final core = Core();
        return core._answer(core._bioStatus()) as Map;
      }),
    );
  }

  /// A call to the phone's own code (MainActivity.kt, AppDelegate.swift),
  /// answered as the core would.
  /// Browsers the person trusted for Android's autofill (`{package, name}`),
  /// and stopping: elsewhere there are none.
  static Future<List<Map<String, String>>> trustedBrowsers() async {
    if (!Platform.isAndroid) return const [];
    final list = await platform.invokeMethod<List>('trustedBrowsers') ?? [];
    return [for (final b in list) Map<String, String>.from(b as Map)];
  }

  static Future<void> untrustBrowser(String package) async {
    if (Platform.isAndroid) {
      await platform.invokeMethod<bool>('untrustBrowser', {'package': package});
    }
  }

  static Future<Map<String, dynamic>> _onPhone(
    String method, [
    Map<String, dynamic>? arguments,
  ]) async => Map<String, dynamic>.from(
    (await platform.invokeMethod<Map>(method, arguments)) ?? {'ok': false},
  );

  /// Starts the desktop agent installed next to the app if it isn't running
  /// (its tray, and Windows Hello). Windows and macOS, where the tray is.
  static Future<Answer> startAgent() =>
      _background((c) => c._answer(c._startAgent()));

  /// Turns it on (the agent asks); `on` says whether the person agreed.
  static Future<Answer> enrollBiometric() async => isPhone
      ? Answer(await _onPhone('biometricEnroll'))
      : _background((c) => c._answer(c._bioEnroll()));

  /// Unlocks with it; `unlocked` is false if the person cancelled. While
  /// unlocked, it's the person confirming it's them (approving a sign-in),
  /// and a phone's prompt says [reason].
  static Future<Answer> unlockWithBiometric({String? reason}) async => isPhone
      ? Answer(await _onPhone('biometricUnlock', {'reason': ?reason}))
      : _background((c) => c._answer(c._bioUnlock()));

  /// Turns it off, for the browser extension too (on a computer).
  static Future<Answer> removeBiometric() async => isPhone
      ? Answer(await _onPhone('biometricRemove'))
      : _background((c) => c._answer(c._bioRemove()));

  /// A sign-in Android's autofill offered to save (name, username, password,
  /// uri), once; null if there's none. The editor opens with it.
  static Future<Map<String, dynamic>?> pendingSave() async {
    if (!Platform.isAndroid) return null;
    final save = await platform.invokeMethod<Map>('pendingSave');
    return save == null ? null : Map<String, dynamic>.from(save);
  }

  /// For the phone tests: debug builds' stand-in for the phone's biometric
  /// prompt, which says yes (Biometric.kt, AppDelegate.swift). False in a
  /// release build.
  static Future<bool> useBiometricStandIn(bool on) async =>
      await platform.invokeMethod<bool>('biometricStandIn', {'on': on}) ??
      false;

  /// For tests: the agent with this home folder and program, instead of the installed one.
  void useAgentHome(String home, String program) =>
      _with([home, program], (p) => _answer(_useAgentHome(p[0], p[1])));

  static Future<Answer> trash(String id) =>
      _background((c) => c._with([id], (p) => c._answer(c._trash(p[0]))));

  /// Makes a folder, renames one, or deletes one (its items stay, in no
  /// folder); each syncs after.
  static Future<Answer> createFolder(String name) => _background(
    (c) => c._with([name], (p) => c._answer(c._createFolder(p[0]))),
  );

  static Future<Answer> renameFolder(String id, String name) => _background(
    (c) => c._with([id, name], (p) => c._answer(c._renameFolder(p[0], p[1]))),
  );

  static Future<Answer> deleteFolder(String id) => _background(
    (c) => c._with([id], (p) => c._answer(c._deleteFolder(p[0]))),
  );

  Map<String, dynamic> lock() => _answer(_lock()) as Map<String, dynamic>;

  Map<String, dynamic> signOut() => _answer(_signOut()) as Map<String, dynamic>;

  /// When to lock by itself: `lockAfterMinutes` (null: only when the app
  /// closes) and `lockOnSleep` (when the screen locks or the computer sleeps).
  Map<String, dynamic> preferences() =>
      _answer(_preferences()) as Map<String, dynamic>;

  /// The organizations' "Vault lock" policy, which Settings keeps to:
  /// `maxMinutes` (the longest wait, or null), `immediately`, `onSystemLock`,
  /// and `signOut`; and `removeUnlockWithPin`, their "Remove Unlock with
  /// PIN" policy, under which the app doesn't offer biometric unlock itself.
  Map<String, dynamic> lockPolicy() =>
      _answer(_lockPolicy()) as Map<String, dynamic>;

  Answer setPreferences(Map<String, dynamic> preferences) => Answer(
    _with([jsonEncode(preferences)], (p) => _answer(_setPreferences(p[0])))
        as Map<String, dynamic>,
  );

  /// The person used the app just now.
  void used() => _answer(_used());

  /// Locks if it's time to (the core decides, from what the OS says):
  /// `locked` says whether the vault is locked now.
  Map<String, dynamic> tick() => _answer(_tick()) as Map<String, dynamic>;

  /// The OS says the computer is about to sleep: locks if the person chose that.
  /// The app left the screen: locks if the person chose "Immediately" (phones).
  Map<String, dynamic> left() => _answer(_left()) as Map<String, dynamic>;

  Map<String, dynamic> sleeping() =>
      _answer(_sleeping()) as Map<String, dynamic>;

  /// Where items live (core/src/places.rs): `folders`, the person's own,
  /// and `organizations`, each with its shared folders (`collections`), each
  /// `{id, name, label, depth, parentId}`, nested by name, and a shared
  /// folder with what the person can do in it (`grid`, `canCreate`).
  Map<String, dynamic> places() => _answer(_places()) as Map<String, dynamic>;

  /// A website's icon for a login's tile (core/src/icons.rs): the image,
  /// or null where there's none, icons are off, or the vault is locked. The
  /// core keeps it in memory while the vault is open; it waits on the
  /// server the first time, on a background isolate.
  static Future<Uint8List?> icon(String host) async {
    final answer = await _background(
      (c) => c._with([host], (p) => c._answer(c._icon(p[0]))),
    );
    final icon = answer.ok ? answer.body['icon'] as String? : null;
    return icon == null ? null : base64Decode(icon);
  }

  /// Websites' icons the core answers [icon] with, without a server: for
  /// tests, with a core built with its test hooks.
  @visibleForTesting
  void testIcons(Map<String, List<int>> icons) => _with([
    jsonEncode({for (final e in icons.entries) e.key: base64Encode(e.value)}),
  ], (p) => _answer(_testIcons(p[0])));

  /// Copies what the generator shows: the core takes it as the generator's
  /// last value, then copies that as it copies any secret, which clears in
  /// 30 seconds (on a phone, through the phone's own code).
  Future<Answer> copyGenerated(String shown) async {
    final kept = Answer(
      _with([shown], (p) => _answer(_keepGenerated(p[0])))
          as Map<String, dynamic>,
    );
    if (!kept.ok) return kept;
    return copy('', 'generated');
  }

  /// How the app looks on this device: `showIcons`.
  Map<String, dynamic> appearance() =>
      _answer(_appearance()) as Map<String, dynamic>;

  Answer setAppearance(Map<String, dynamic> appearance) => Answer(
    _with([jsonEncode(appearance)], (p) => _answer(_setAppearance(p[0])))
        as Map<String, dynamic>,
  );

  /// The logins with something to fix (core/src/health.rs): `items`, each
  /// `{id, weak, reused, unsecured}`. Judged on a background isolate.
  static Future<Answer> health() => _background((c) => c._answer(c._health()));

  /// An item's past versions (core/src/history.rs), from the server:
  /// `{hidePasswords, canRestore, canClear, unreadable, versions: [{id,
  /// revisionDate, savedBy, opens, restorable, changes, rows}]}`.
  static Future<Answer> history(String id) =>
      _background((c) => c._with([id], (p) => c._answer(c._history(p[0]))));

  /// One version beside the item now (`rows`, each `{label, version,
  /// current, secret, changed}`), its secrets shown when [reveal].
  Answer itemVersion(String id, String versionId, {bool reveal = false}) =>
      Answer(
        _with(
          [id, versionId, '$reveal'],
          (p) => _answer(_itemVersion(p[0], p[1], p[2])),
        ) as Map<String, dynamic>,
      );

  /// Restores a version, as a normal edit, then syncs.
  static Future<Answer> restoreVersion(String id, String versionId) =>
      _background(
        (c) => c._with([
          id,
          versionId,
        ], (p) => c._answer(c._restoreVersion(p[0], p[1]))),
      );

  /// Clears an item's history (no version), or deletes one version.
  static Future<Answer> deleteHistory(String id, [String? versionId]) =>
      _background(
        (c) => c._with([
          id,
          versionId ?? '',
        ], (p) => c._answer(c._deleteHistory(p[0], p[1]))),
      );

  /// Favorites an item or not, then syncs.
  static Future<Answer> setFavorite(String id, bool favorite) => _background(
    (c) => c._with([
      id,
      '$favorite',
    ], (p) => c._answer(c._setFavorite(p[0], p[1]))),
  );

  /// Opens a website in the browser: only http and https. Phones and the
  /// Mac through their own code, Windows and Linux through the OS's opener.
  static Future<bool> openWebsite(String address) async {
    final text = address.trim();
    final uri = Uri.tryParse(text.contains('://') ? text : 'https://$text');
    if (uri == null || (uri.scheme != 'https' && uri.scheme != 'http')) {
      return false;
    }
    final url = uri.toString();
    if (isPhone || Platform.isMacOS) {
      return await platform.invokeMethod<bool>('openUrl', {'url': url}) ??
          false;
    }
    try {
      final opened = Platform.isWindows
          ? await Process.run('rundll32', ['url.dll,FileProtocolHandler', url])
          : await Process.run('xdg-open', [url]);
      return opened.exitCode == 0;
    } on ProcessException {
      return false;
    }
  }

  /// Templates for New item, grouped as the web vault groups them
  /// (core/src/templates.rs): `[{name, templates: [{id, name, base,
  /// category, fields: [{label, kind, value}]}]}]`.
  List<Map<String, dynamic>> templates() =>
      (_answer(_templates()) as List).cast<Map<String, dynamic>>();

  List<Map<String, dynamic>> list(String query) =>
      (_with([query], (p) => _answer(_list(p[0]))) as List)
          .cast<Map<String, dynamic>>();

  Map<String, dynamic>? item(String id) =>
      _with([id], (p) => _answer(_item(p[0]))) as Map<String, dynamic>?;

  /// A secret, while the person reveals it or edits: `password`, `notes`, `totp`, or `field:<index>`.
  String? reveal(String id, String what) =>
      _with([id, what], (p) => _answer(_reveal(p[0], p[1]))) as String?;

  /// Copies without the value passing through Dart; secrets clear after 30
  /// seconds. On phones the app's own code (Kotlin, Swift) takes it from the
  /// core to the clipboard.
  Future<Answer> copy(String id, String what) async {
    if (isPhone) {
      final copied = await platform.invokeMethod<bool>('copy', {
        'id': id,
        'what': what,
      });
      return Answer({
        'ok': copied == true,
        if (copied != true) 'error': 'Couldn\'t copy that. Try again.',
      });
    }
    return Answer(
      _with([id, what], (p) => _answer(_copy(p[0], p[1])))
          as Map<String, dynamic>,
    );
  }

  Map<String, dynamic>? totp(String id) =>
      _with([id], (p) => _answer(_totp(p[0]))) as Map<String, dynamic>?;

  /// The generator's last choices on this device (core/src/generator.rs):
  /// `kind` (`password` or `passphrase`), `length`, `uppercase`,
  /// `lowercase`, `digits`, `symbols`, `avoidAmbiguous`, `words`,
  /// `separator`, `capitalize`, and `includeNumber`.
  Map<String, dynamic> generatorOptions() =>
      _answer(_generatorOptions()) as Map<String, dynamic>;

  /// The organizations' "Password generator" policy: `kind` (the only kind
  /// allowed, or null), `minLength`, `uppercase`, `lowercase`, `digits`,
  /// `symbols`, `minDigits`, `minSymbols`, `minWords`, `capitalize`, and
  /// `includeNumber`.
  Map<String, dynamic> generatorPolicy() =>
      _answer(_generatorPolicy()) as Map<String, dynamic>;

  /// A new password or passphrase for the editor's field: `{ok, password,
  /// options}`, with the choices held to the organizations' policy, or the
  /// words to show. Without options, the last choices; the choices become
  /// the next ones.
  Answer generate([Map<String, dynamic>? options]) => Answer(
    _with([
      options == null ? 'null' : jsonEncode(options),
    ], (p) => _answer(_generate(p[0]))) as Map<String, dynamic>,
  );
}
