// The core (Rust), through its C bridge (core/src/ffi.rs). No key crosses
// here, and a secret only when the person reveals it or edits it
// (docs/core.md, "Memory"). Slow calls (signing in, unlocking, syncing,
// saving) run on a background isolate; the screens don't read while one runs.
//
// Several accounts can be signed in at once (DESIGN.md in the meta repo,
// "Several accounts in the apps"). Every call about an account names the one
// the screens show ([Core.account]) as it was when the call was made, so a
// call can't land on another account the screens switched to meanwhile.
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
typedef _Str4 = Pointer<Utf8> Function(
  Pointer<Utf8>,
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

/// Android, where the app helps turn on its autofill service and passkey
/// provider (autofill_offer.dart).
bool get isAndroid => debugAndroid ?? Platform.isAndroid;

/// Tests only: Android's autofill setup on a computer, with a stand-in for
/// the phone's own code. Null is what this is.
@visibleForTesting
bool? debugAndroid;

/// An answer from a call that can fail: ok, or the core's words (plain
/// words with what to do next; core/src/error.rs).
class Answer {
  Answer(Map<String, dynamic> json)
    : ok = json['ok'] == true,
      error = json['error'] as String?,
      signedOut = json['signedOut'] == true,
      offline = json['offline'] == true,
      aboutTheServer = json['field'] == 'server',
      body = json;

  final bool ok;
  final String? error;
  final bool signedOut;

  /// This device couldn't reach the server: offline, or the server is down.
  final bool offline;

  /// It's about the server's address (it can't be reached, or isn't a
  /// PassBundle server), not the email or master password: the sign-in
  /// screen says it under the server field.
  final bool aboutTheServer;
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
  late final _bioStatus = _lib.lookupFunction<_Str1, _Str1>('pb_biometric_status');
  late final _bioEnroll = _lib.lookupFunction<_Str1, _Str1>('pb_biometric_enroll');
  late final _bioUnlock = _lib.lookupFunction<_Str1, _Str1>('pb_biometric_unlock');
  late final _bioRemove = _lib.lookupFunction<_Str1, _Str1>('pb_biometric_remove');
  late final _bioConfirmApproval = _lib.lookupFunction<_Str1, _Str1>('pb_confirm_approval_with_biometric');
  late final _state = _lib.lookupFunction<_Str1, _Str1>('pb_state');
  late final _servers = _lib.lookupFunction<_Str0, _Str0>('pb_servers');
  late final _signIn = _lib.lookupFunction<_Str3, _Str3>('pb_sign_in');
  late final _checkServer = _lib.lookupFunction<_Str1, _Str1>(
    'pb_check_server',
  );
  late final _passwordHint = _lib.lookupFunction<_Str1, _Str1>(
    'pb_password_hint',
  );
  late final _signUp = _lib.lookupFunction<_Str5, _Str5>('pb_sign_up');
  late final _finish = _lib.lookupFunction<_FinishNative, _Finish>(
    'pb_finish_sign_in',
  );
  late final _sendEmail = _lib.lookupFunction<_Str0, _Str0>(
    'pb_send_email_code',
  );
  late final _newDeviceStatus = _lib.lookupFunction<_Str0, _Str0>(
    'pb_new_device_status',
  );
  late final _newDeviceEmail = _lib.lookupFunction<_Str0, _Str0>(
    'pb_new_device_email',
  );
  late final _newDeviceCode = _lib.lookupFunction<_Str1, _Str1>(
    'pb_new_device_code',
  );
  late final _duoStart = _lib.lookupFunction<_Str0, _Str0>('pb_duo_start');
  late final _duoFinish = _lib
      .lookupFunction<
        Pointer<Utf8> Function(Pointer<Utf8>, Bool),
        Pointer<Utf8> Function(Pointer<Utf8>, bool)
      >('pb_duo_finish');
  late final _securityKey = _lib.lookupFunction<_FlagNative, _Flag>(
    'pb_security_key_sign_in',
  );
  late final _securityKeyCancel = _lib.lookupFunction<_Str0, _Str0>(
    'pb_security_key_cancel',
  );
  late final _handOverLink = _lib.lookupFunction<_Str1, _Str1>(
    'pb_hand_over_link',
  );
  late final _handedOverLink = _lib.lookupFunction<_Str0, _Str0>(
    'pb_handed_over_link',
  );
  late final _unlock = _lib.lookupFunction<_Str2, _Str2>('pb_unlock');
  late final _saveAttachment = _lib.lookupFunction<_Str3, _Str3>('pb_save_attachment');
  late final _confirmMasterPassword = _lib.lookupFunction<_Str3, _Str3>('pb_confirm_master_password');
  late final _signInRequests = _lib.lookupFunction<_Str1, _Str1>('pb_sign_in_requests');
  late final _answerSignInRequest = _lib.lookupFunction<_Str4, _Str4>('pb_answer_sign_in_request');
  late final _answerNewDeviceCheck = _lib.lookupFunction<_Str4, _Str4>('pb_answer_new_device_check');
  late final _confirmWithMasterPassword = _lib.lookupFunction<_Str2, _Str2>('pb_confirm_with_master_password');
  late final _approvals = _lib.lookupFunction<_Str1, _Str1>('pb_approvals');
  late final _setApprovals = _lib.lookupFunction<_Str2, _Str2>('pb_set_approvals');
  late final _sync = _lib.lookupFunction<_Str1, _Str1>('pb_sync');
  late final _syncSettings = _lib.lookupFunction<_Str1, _Str1>('pb_sync_settings');
  late final _lock = _lib.lookupFunction<_Str1, _Str1>('pb_lock');
  late final _signOut = _lib.lookupFunction<_Str1, _Str1>('pb_sign_out');
  late final _preferences = _lib.lookupFunction<_Str1, _Str1>('pb_preferences');
  late final _lockPolicy = _lib.lookupFunction<_Str1, _Str1>('pb_lock_policy');
  late final _setPreferences = _lib.lookupFunction<_Str2, _Str2>('pb_set_preferences');
  late final _used = _lib.lookupFunction<_Str1, _Str1>('pb_used');
  late final _tick = _lib.lookupFunction<_Str0, _Str0>('pb_tick');
  late final _sleeping = _lib.lookupFunction<_Str0, _Str0>('pb_sleeping');
  late final _left = _lib.lookupFunction<_Str0, _Str0>('pb_left');
  late final _places = _lib.lookupFunction<_Str1, _Str1>('pb_places');
  late final _templates = _lib.lookupFunction<_Str1, _Str1>('pb_templates');
  late final _icon = _lib.lookupFunction<_Str2, _Str2>('pb_icon');
  late final _testIcons = _lib.lookupFunction<_Str1, _Str1>('pb_test_icons');
  late final _keepGenerated = _lib.lookupFunction<_Str2, _Str2>('pb_keep_generated');
  late final _appearance = _lib.lookupFunction<_Str0, _Str0>('pb_appearance');
  late final _setAppearance = _lib.lookupFunction<_Str1, _Str1>(
    'pb_set_appearance',
  );
  late final _screenshots = _lib.lookupFunction<_Str0, _Str0>(
    'pb_screenshots',
  );
  late final _setScreenshots = _lib.lookupFunction<_Str1, _Str1>(
    'pb_set_screenshots',
  );
  late final _health = _lib.lookupFunction<_Str1, _Str1>('pb_health');
  late final _reminders = _lib.lookupFunction<_Str1, _Str1>('pb_reminders');
  late final _testExpiration = _lib.lookupFunction<_Str1, _Str1>(
    'pb_test_expiration',
  );
  late final _browserProfiles = _lib.lookupFunction<_Str1, _Str1>(
    'pb_browser_profiles',
  );
  late final _browserImportOffer = _lib.lookupFunction<_Str1, _Str1>(
    'pb_browser_import_offer',
  );
  late final _browserImport = _lib.lookupFunction<_Str2, _Str2>(
    'pb_browser_import',
  );
  late final _browserTurnOff = _lib.lookupFunction<_Str2, _Str2>(
    'pb_browser_turn_off',
  );
  late final _testBrowsers = _lib.lookupFunction<_Str3, _Str3>(
    'pb_test_browsers',
  );
  late final _testBrowsersRead = _lib.lookupFunction<_Str0, _Str0>(
    'pb_test_browsers_read',
  );
  late final _history = _lib.lookupFunction<_Str2, _Str2>('pb_history');
  late final _itemVersion = _lib.lookupFunction<_Str4, _Str4>('pb_item_version');
  late final _restoreVersion = _lib.lookupFunction<_Str3, _Str3>('pb_restore_version');
  late final _deleteHistory = _lib.lookupFunction<_Str3, _Str3>('pb_delete_history');
  late final _setFavorite = _lib.lookupFunction<_Str3, _Str3>('pb_set_favorite');
  late final _list = _lib.lookupFunction<_Str2, _Str2>('pb_list');
  late final _item = _lib.lookupFunction<_Str2, _Str2>('pb_item');
  late final _reveal = _lib.lookupFunction<_Str3, _Str3>('pb_reveal');
  late final _copy = _lib.lookupFunction<_Str3, _Str3>('pb_copy');
  late final _totp = _lib.lookupFunction<_Str2, _Str2>('pb_totp');
  late final _save = _lib.lookupFunction<_Str3, _Str3>('pb_save');
  late final _trash = _lib.lookupFunction<_Str2, _Str2>('pb_trash');
  late final _createFolder = _lib.lookupFunction<_Str2, _Str2>('pb_create_folder');
  late final _renameFolder = _lib.lookupFunction<_Str3, _Str3>('pb_rename_folder');
  late final _deleteFolder = _lib.lookupFunction<_Str2, _Str2>('pb_delete_folder');
  late final _generate = _lib.lookupFunction<_Str2, _Str2>('pb_generate');
  late final _generatorOptions = _lib.lookupFunction<_Str1, _Str1>('pb_generator_options');
  late final _generatorPolicy = _lib.lookupFunction<_Str1, _Str1>('pb_generator_policy');
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
  late final _accounts = _lib.lookupFunction<_Str0, _Str0>('pb_accounts');
  late final _listAll = _lib.lookupFunction<_Str1, _Str1>('pb_list_all');
  late final _setAccountLabel = _lib.lookupFunction<_Str2, _Str2>(
    'pb_set_account_label',
  );
  late final _switch = _lib.lookupFunction<_Str1, _Str1>('pb_switch');
  late final _cancelSignIn = _lib.lookupFunction<_Str0, _Str0>(
    'pb_cancel_sign_in',
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

  /// Runs a slow call on a background isolate (the core is the same library
  /// there), for the account the screens show now ([account]), whichever
  /// they show by the time it runs.
  static Future<Answer> _background(
    dynamic Function(Core core, String account) call,
  ) async {
    final account = Core.account;
    return Answer(
      Map<String, dynamic>.from(
        await Isolate.run(() => call(Core(), account) as Map),
      ),
    );
  }

  /// The account the screens show, by its id on this device: every call
  /// about an account is for this one, as it is when the call is made. Empty
  /// while none is signed in, or one is being added.
  static String account = '';

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
  /// the refresh tokens in memory for tests and tools. The screens show the
  /// account shown last ([accounts]).
  Map<String, dynamic> openAt(String dir, {bool memorySecrets = true}) {
    folder = dir;
    final opened =
        _with([dir], (p) => _answer(_openAt(p[0], memorySecrets)))
            as Map<String, dynamic>;
    account = opened['active'] as String? ?? '';
    return opened;
  }

  /// The accounts signed in here: `active` (the one the screens show, or
  /// null), `accounts` (each `{id, server, email, state}`, in the order they
  /// were added), `most` (how many may be), and `last` (`{server, email}`
  /// last signed in to or out of); with the state of the active one
  /// (`state`, `server`, `email`).
  Map<String, dynamic> accounts() =>
      _answer(_accounts()) as Map<String, dynamic>;

  /// The screens show this account now; the one they showed stays as it
  /// is. Answers as [accounts].
  Answer switchTo(String id) {
    final answer = Answer(
      _with([id], (p) => _answer(_switch(p[0]))) as Map<String, dynamic>,
    );
    if (answer.body['active'] is String) {
      account = answer.body['active'] as String;
    }
    return answer;
  }

  /// Names an account ([id], or the one the screens show) and picks its
  /// color: [name] (empty for its email) and [tone], one of [accounts]'
  /// `tones` (null keeps its color). Kept on this device only. Answers as
  /// [accounts], or the words to show.
  Answer setAccountLabel(String name, String? tone, [String? id]) => Answer(
    _with([
          id ?? account,
          jsonEncode({'name': name, 'tone': tone}),
        ], (p) => _answer(_setAccountLabel(p[0], p[1])))
        as Map<String, dynamic>,
  );

  /// What a search finds in every unlocked account (the "All accounts"
  /// place), by name: each item as [list] has it, with its `account`.
  List<Map<String, dynamic>> listAll(String query) =>
      (_with([query], (p) => _answer(_listAll(p[0]))) as List? ?? const [])
          .cast<Map<String, dynamic>>();

  /// Gives up a sign-in waiting for its second step, and the account being
  /// added. Answers as [accounts].
  Map<String, dynamic> cancelSignIn() =>
      _answer(_cancelSignIn()) as Map<String, dynamic>;

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

  /// The account's "signedOut", "locked", or "unlocked", with "server" and
  /// "email"; for none, "signedOut" with the server and email last signed
  /// in to or out of.
  Map<String, dynamic> state([String? id]) =>
      _with([id ?? account], (p) => _answer(_state(p[0])))
          as Map<String, dynamic>;

  /// Signs in: to an account here already with the same server and email,
  /// or a new one, which joins the others. Once it's done, the screens show
  /// it ([account]).
  static Future<Answer> signIn(String server, String email, String password) =>
      _signedIn(
        _background(
          (c, _) => c._with([
            server,
            email,
            password,
          ], (p) => c._answer(c._signIn(p[0], p[1], p[2]))),
        ),
      );

  /// Whether a PassBundle server answers at an address, before signing in or
  /// making an account there (docs/core.md, "Checking a server"): ok, or
  /// what's wrong, [Answer.aboutTheServer].
  static Future<Answer> checkServer(String server) => _background(
    (c, _) => c._with([server], (p) => c._answer(c._checkServer(p[0]))),
  );

  /// The master password hint of the account shown, for the lock screen: its
  /// server emails it (`hint` null), or shows it (`hint`), as it's set to.
  static Future<Answer> passwordHint() => _background(
    (c, a) => c._with([a], (p) => c._answer(c._passwordHint(p[0]))),
  );

  /// A sign-in that's done names the account signed in to: the screens show it.
  static Future<Answer> _signedIn(Future<Answer> signing) async {
    final answer = await signing;
    final signedIn = answer.body['account'];
    if (answer.ok && signedIn is String) account = signedIn;
    return answer;
  }

  /// Makes an account on the server, then signs in to it (docs/core.md,
  /// "Making an account"). [name] and [hint] may be empty.
  static Future<Answer> signUp(
    String server,
    String email,
    String name,
    String password,
    String hint,
  ) => _signedIn(
    _background(
      (c, _) => c._with([
        server,
        email,
        name,
        password,
        hint,
      ], (p) => c._answer(c._signUp(p[0], p[1], p[2], p[3], p[4]))),
    ),
  );

  static Future<Answer> finishSignIn(
    int provider,
    String code,
    bool remember,
  ) => _signedIn(
    _background(
      (c, _) => c._with([
        code,
      ], (p) => c._answer(c._finish(provider, p[0], remember))),
    ),
  );

  static Future<Answer> sendEmailCode() =>
      _background((c, _) => c._answer(c._sendEmail()));

  /// How the new-device check holding the sign-in stands now (new_device.dart,
  /// core/src/new_device.rs), asked every 3 seconds while it waits: as
  /// [signIn] answers, `newDevice` with its `check`, or `done` once another
  /// device approved it and the sign-in finished.
  static Future<Answer> newDeviceStatus() =>
      _signedIn(_background((c, _) => c._answer(c._newDeviceStatus())));

  /// "Email me a code" for the sign-in the new-device check holds (or, when
  /// other devices can approve it, the code after a day's wait): as [signIn]
  /// answers, with the check as it stands now.
  static Future<Answer> newDeviceEmail() =>
      _background((c, _) => c._answer(c._newDeviceEmail()));

  /// Finishes the sign-in the new-device check holds with the code from the
  /// email: as [signIn] answers.
  static Future<Answer> newDeviceCode(String code) => _signedIn(
    _background(
      (c, _) => c._with([code], (p) => c._answer(c._newDeviceCode(p[0]))),
    ),
  );

  /// Duo's address for the sign-in waiting for its second step, for the
  /// browser (two_step.dart): `{ok, url}`, or a sign-in that went through
  /// without it, as [signIn] answers.
  static Future<Answer> duoStart() =>
      _signedIn(_background((c, _) => c._answer(c._duoStart())));

  /// The second step with Duo's answer: the link the browser came back with.
  static Future<Answer> duoFinish(String link, bool remember) => _signedIn(
    _background(
      (c, _) =>
          c._with([link], (p) => c._answer(c._duoFinish(p[0], remember))),
    ),
  );

  /// The second step with a security key: the core asks the key (this
  /// device's own dialog on Windows, a key plugged in on Linux and the Mac,
  /// one held to the phone or plugged in on Android), as long as the
  /// server's request says or until [securityKeyCancel].
  static Future<Answer> securityKeySignIn(bool remember) =>
      _signedIn(_background((c, _) => c._answer(c._securityKey(remember))));

  /// Stops a security key's wait ([securityKeySignIn]).
  void securityKeyCancel() => _answer(_securityKeyCancel());

  /// A second copy of the app that the OS opened with Duo's link (Windows
  /// and Linux) leaves it for the copy that's signing in: whether it was one.
  bool handOverLink(String link) =>
      (_with([link], (p) => _answer(_handOverLink(p[0]))) as Map)['ok'] == true;

  /// The link a second copy left ([handOverLink]), once, or null.
  String? handedOverLink() => _answer(_handedOverLink()) as String?;

  static Future<Answer> unlock(String password) => _background(
    (c, a) => c._with([a, password], (p) => c._answer(c._unlock(p[0], p[1]))),
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
            'account': account,
            'id': id,
            'attachmentId': attachmentId,
            'name': name,
          }),
        )
      : _background(
          (c, a) => c._with([
            a,
            id,
            attachmentId,
          ], (p) => c._answer(c._saveAttachment(p[0], p[1], p[2]))),
        );

  /// The master password, for an item that asks for it again (docs/core.md,
  /// "Asking for the master password again"): then its secrets may be used.
  static Future<Answer> confirmMasterPassword(String id, String password) =>
      _background(
        (c, a) => c._with([
          a,
          id,
          password,
        ], (p) => c._answer(c._confirmMasterPassword(p[0], p[1], p[2]))),
      );

  static Future<Answer> sync() =>
      _background((c, a) => c._with([a], (p) => c._answer(c._sync(p[0]))));

  /// Sign-ins waiting for an account's answer (docs/core.md, "Approving a
  /// sign-in"), from the server: `requests`, each `{id, device, ip,
  /// fingerprint, ageSeconds, secondsLeft}`, newest first; `checks`, sign-ins
  /// on new devices waiting for their number, each `{id, device, deviceType,
  /// ip, location, ageSeconds, secondsLeft, emailAfter}`; and
  /// `locationSource` (`{name, url}`, or null), who the locations are from.
  /// The account the screens show, or [id].
  static Future<Answer> signInRequests([String? id]) {
    final asked = id;
    return _background(
      (c, a) =>
          c._with([asked ?? a], (p) => c._answer(c._signInRequests(p[0]))),
    );
  }

  /// Approves or denies one, for the account it was listed for ([id], or
  /// the one the screens show). Approving takes the phrase the person was
  /// shown, and may answer `confirmFirst`: confirm it's them, then again.
  static Future<Answer> answerSignInRequest(
    String requestId,
    String fingerprint,
    bool approve, [
    String? id,
  ]) {
    final asked = id;
    return _background(
      (c, a) => c._with([
        asked ?? a,
        requestId,
        fingerprint,
        approve ? 'true' : 'false',
      ], (p) => c._answer(c._answerSignInRequest(p[0], p[1], p[2], p[3]))),
    );
  }

  /// Approves another device's sign-in that the new-device check holds,
  /// with the [number] the person typed (two digits), or denies it (null),
  /// for the account it was listed for ([id], or the one the screens show):
  /// `answered` is "approved", "denied", or "wrongNumber" (it wasn't the new
  /// device's number, so the server denied it). Sends no key, so it asks
  /// for no confirmation.
  static Future<Answer> answerNewDeviceCheck(
    String checkId,
    String? number, [
    String? id,
  ]) {
    final asked = id;
    return _background(
      (c, a) => c._with([
        asked ?? a,
        checkId,
        number ?? '',
        number == null ? 'false' : 'true',
      ], (p) => c._answer(c._answerNewDeviceCheck(p[0], p[1], p[2], p[3]))),
    );
  }

  /// The master password, as the person confirming it's them to approve a
  /// sign-in to an account ([id], or the one the screens show).
  static Future<Answer> confirmWithMasterPassword(
    String password, [
    String? id,
  ]) {
    final asked = id;
    return _background(
      (c, a) => c._with([
        asked ?? a,
        password,
      ], (p) => c._answer(c._confirmWithMasterPassword(p[0], p[1]))),
    );
  }

  /// Whether approving a sign-in asks for biometrics or the master password
  /// first, for an account on this device ([id], or the one the screens
  /// show).
  bool confirmApprovals([String? id]) =>
      (_with([id ?? account], (p) => _answer(_approvals(p[0]))) as Map?)?['confirm'] !=
      false;

  Answer setConfirmApprovals(bool on, [String? id]) => Answer(
    _with([
          id ?? account,
          on ? 'true' : 'false',
        ], (p) => _answer(_setApprovals(p[0], p[1])))
        as Map<String, dynamic>,
  );

  /// Saves the generator's settings to the server, so they're the same on
  /// the person's other devices (core/src/synced_settings.rs).
  static Future<Answer> syncSettings() => _background(
    (c, a) => c._with([a], (p) => c._answer(c._syncSettings(p[0]))),
  );

  /// Saves an item from the editor: `id` (the new one's, for a new item),
  /// and `synced`, false when the sync after saving didn't come through
  /// (it's saved all the same, and shows here; core/src/app.rs, `save`).
  static Future<Answer> save(String? id, Map<String, dynamic> draft) =>
      _background(
        (c, a) => c._with([
          a,
          id ?? '',
          jsonEncode(draft),
        ], (p) => c._answer(c._save(p[0], p[1], p[2]))),
      );

  /// Unlocking with Windows Hello or Touch ID, through the desktop agent, or
  /// on a phone with its own biometrics, through the app's Kotlin or Swift:
  /// `status` is setUp, notSetUp, unavailable, or noAgent, and `method` its name.
  static Future<Map<String, dynamic>> biometricStatus([String? id]) async {
    final asked = id ?? account;
    if (isPhone) return _onPhone('biometricStatus', {'account': asked});
    return Map<String, dynamic>.from(
      await Isolate.run(() {
        final core = Core();
        return core._with([asked], (p) => core._answer(core._bioStatus(p[0])))
            as Map;
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

  /// Whether PassBundle is Android's autofill service and, on Android 14 and
  /// later, a passkey provider (AutofillSetup.kt): `apiLevel`,
  /// `autofillSupported`, `autofillOn`, `passkeysAvailable`, `passkeysOn`.
  /// Null elsewhere.
  static Future<Map<String, dynamic>?> autofillStatus() async {
    if (!isAndroid) return null;
    final status = await platform.invokeMethod<Map>('autofillStatus');
    return status == null ? null : Map<String, dynamic>.from(status);
  }

  /// Android's own prompt to use PassBundle for autofill, or where it's
  /// missing, the settings it's chosen in: which opened (`prompt`,
  /// `providers`, `settings`), or null if none did.
  static Future<String?> turnOnAutofill() async =>
      isAndroid ? await platform.invokeMethod<String>('turnOnAutofill') : null;

  /// Android 14 and later's list of passkey providers: true if it opened.
  static Future<bool> openPasskeySettings() async =>
      isAndroid &&
      (await platform.invokeMethod<bool>('openPasskeySettings') ?? false);

  /// For the phone tests: debug builds record what turning on autofill and
  /// passkeys would open, instead of opening Android's prompt
  /// (AutofillSetup.kt). False in a release build.
  static Future<bool> useAutofillSetupStandIn(bool on) async =>
      await platform.invokeMethod<bool>('autofillSetupStandIn', {'on': on}) ??
      false;

  /// What the stand-in recorded, in order: `for` (autofill or passkeys),
  /// then an intent's `action`, `data`, and `forResult`, or a pending
  /// intent's `creator` and `activity`.
  static Future<List<Map<String, dynamic>>> autofillSetupOpened() async => [
    for (final o
        in await platform.invokeMethod<List>('autofillSetupOpened') ?? [])
      Map<String, dynamic>.from(o as Map),
  ];

  static Future<Map<String, dynamic>> _onPhone(
    String method, [
    Map<String, dynamic>? arguments,
  ]) async => Map<String, dynamic>.from(
    (await platform.invokeMethod<Map>(method, arguments)) ?? {'ok': false},
  );

  /// Starts the desktop agent installed next to the app if it isn't running
  /// (its tray, and Windows Hello). Windows and macOS, where the tray is.
  static Future<Answer> startAgent() =>
      _background((c, _) => c._answer(c._startAgent()));

  /// Turns it on for the account the screens show (the agent asks, or the
  /// phone); `on` says whether the person agreed.
  static Future<Answer> enrollBiometric() async => isPhone
      ? Answer(await _onPhone('biometricEnroll', {'account': account}))
      : _background(
          (c, a) => c._with([a], (p) => c._answer(c._bioEnroll(p[0]))),
        );

  /// Unlocks that account with it; `unlocked` is false if the person cancelled.
  static Future<Answer> unlockWithBiometric() async => isPhone
      ? Answer(await _onPhone('biometricUnlock', {'account': account}))
      : _background(
          (c, a) => c._with([a], (p) => c._answer(c._bioUnlock(p[0]))),
        );

  /// The same biometrics, as the person confirming it's them to approve a
  /// sign-in to an account ([id], or the one the screens show), or to turn
  /// off asking for that, and nothing else: its prompt says "Approve
  /// sign-in" where the OS shows the app's words, and the core only compares
  /// the key with that account's open vault's. `confirmed` is false if the
  /// person cancelled.
  static Future<Answer> confirmApprovalWithBiometric([String? id]) async {
    final asked = id;
    return isPhone
        ? Answer(
            await _onPhone('biometricConfirmApproval', {
              'account': asked ?? account,
            }),
          )
        : _background(
            (c, a) => c._with([
              asked ?? a,
            ], (p) => c._answer(c._bioConfirmApproval(p[0]))),
          );
  }

  /// Turns it off for the account the screens show, for the browser
  /// extension too (on a computer).
  static Future<Answer> removeBiometric() async => isPhone
      ? Answer(await _onPhone('biometricRemove', {'account': account}))
      : _background(
          (c, a) => c._with([a], (p) => c._answer(c._bioRemove(p[0]))),
        );

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

  static Future<Answer> trash(String id) => _background(
    (c, a) => c._with([a, id], (p) => c._answer(c._trash(p[0], p[1]))),
  );

  /// Makes a folder, renames one, or deletes one (its items stay, in no
  /// folder); each syncs after.
  static Future<Answer> createFolder(String name) => _background(
    (c, a) =>
        c._with([a, name], (p) => c._answer(c._createFolder(p[0], p[1]))),
  );

  static Future<Answer> renameFolder(String id, String name) => _background(
    (c, a) => c._with([
      a,
      id,
      name,
    ], (p) => c._answer(c._renameFolder(p[0], p[1], p[2]))),
  );

  static Future<Answer> deleteFolder(String id) => _background(
    (c, a) => c._with([a, id], (p) => c._answer(c._deleteFolder(p[0], p[1]))),
  );

  /// Locks an account ([id], or the one the screens show); the others stay
  /// as they are. Answers its state.
  Map<String, dynamic> lock([String? id]) =>
      _with([id ?? account], (p) => _answer(_lock(p[0])))
          as Map<String, dynamic>;

  /// Signs out of an account ([id], or the one the screens show): its
  /// vault, session, and biometric unlock are forgotten, and the others stay
  /// as they are. Answers as [accounts]; the screens show the first of the
  /// others, if any.
  Map<String, dynamic> signOut([String? id]) {
    final answer =
        _with([id ?? account], (p) => _answer(_signOut(p[0])))
            as Map<String, dynamic>;
    account = answer['active'] as String? ?? '';
    return answer;
  }

  /// When to lock by itself: `lockAfterMinutes` (null: only when the app
  /// closes) and `lockOnSleep` (when the screen locks or the computer sleeps).
  Map<String, dynamic> preferences() =>
      _with([account], (p) => _answer(_preferences(p[0])))
          as Map<String, dynamic>;

  /// The organizations' "Vault lock" policy, which Settings keeps to:
  /// `maxMinutes` (the longest wait, or null), `immediately`, `onSystemLock`,
  /// and `signOut`; and `removeUnlockWithPin`, their "Remove Unlock with
  /// PIN" policy, under which the app doesn't offer biometric unlock itself.
  Map<String, dynamic> lockPolicy() =>
      _with([account], (p) => _answer(_lockPolicy(p[0])))
          as Map<String, dynamic>;

  Answer setPreferences(Map<String, dynamic> preferences) => Answer(
    _with([
          account,
          jsonEncode(preferences),
        ], (p) => _answer(_setPreferences(p[0], p[1])))
        as Map<String, dynamic>,
  );

  /// The person used the app just now, in the account the screens show.
  void used() => _with([account], (p) => _answer(_used(p[0])));

  /// Locks each account whose time is up (the core decides, from what the
  /// OS says, by each account's choices and policy): `locked` says whether
  /// one locked just now, and `unlocked` whether any still is; with the
  /// accounts as they are now, as [accounts] (`busy` while a slow call has
  /// the core).
  Map<String, dynamic> tick() => _answer(_tick()) as Map<String, dynamic>;

  /// The app left the screen: each account locks if its person chose
  /// "Immediately" (phones), or its policy says so. `locked` says whether one did.
  Map<String, dynamic> left() => _answer(_left()) as Map<String, dynamic>;

  /// The OS says the computer is about to sleep: each account locks if its
  /// person chose that, or its policy says so. `locked` says whether one did.
  Map<String, dynamic> sleeping() =>
      _answer(_sleeping()) as Map<String, dynamic>;

  /// Where items live (core/src/places.rs): `folders`, the person's own,
  /// and `organizations`, each with its shared folders (`collections`), each
  /// `{id, name, label, depth, parentId}`, nested by name, and a shared
  /// folder with what the person can do in it (`grid`, `canCreate`).
  Map<String, dynamic> places() =>
      _with([account], (p) => _answer(_places(p[0])))
          as Map<String, dynamic>;

  /// A website's icon for a login's tile (core/src/icons.rs): the image,
  /// or null where there's none, icons are off, or the vault is locked. The
  /// core keeps it in memory while the vault is open; it waits on the
  /// server the first time, on a background isolate. Asked of the login's
  /// own account's server ([id], or the one the screens show), never
  /// another's.
  static Future<Uint8List?> icon(String host, [String? id]) async {
    final asked = id;
    final answer = await _background(
      (c, a) => c._with([
        asked ?? a,
        host,
      ], (p) => c._answer(c._icon(p[0], p[1]))),
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

  /// Items given an expiration date (`expires`, "YYYY-MM-DD") and reminder
  /// days (`remind`, as `keykeeper.remind` holds them), by id, whenever a
  /// vault opens: for tests, with a core built with its test hooks, since
  /// the sample vault is encrypted. Unlock after giving them.
  @visibleForTesting
  void testExpiration(Map<String, Map<String, String>> items) =>
      _with([jsonEncode(items)], (p) => _answer(_testExpiration(p[0])));

  // Importing from this computer's browsers (browser_import.dart; the
  // core's browser_import.rs). Computers only.

  /// The browser profiles here, chosen by the rules for the account shown
  /// (`profiles`, `guided`, `ask`, `keepsItemsOut`). Reads no passwords.
  /// Away from the screens, since a sync may have the core.
  static Future<Answer> browserProfiles() => _background(
    (c, a) => c._with([a], (p) => c._answer(c._browserProfiles(p[0]))),
  );

  /// Whether to offer importing from the browsers by itself now: once per
  /// account on this device, when there's a profile to import from. Saying
  /// yes counts as offered.
  static Future<bool> browserImportOffer() async =>
      (await _background(
        (c, a) =>
            c._with([a], (p) => c._answer(c._browserImportOffer(p[0]))),
      )).body['offer'] ==
      true;

  /// Imports from the profiles the person chose just now ([ids]), with the
  /// Firefox Primary Passwords they typed for this import, by profile id.
  /// The bridge's copies are wiped after the call; the request's Dart string
  /// can't be, as with the master password (docs/core.md, "Memory").
  static Future<Answer> browserImport(
    List<String> ids, [
    Map<String, String> primaryPasswords = const {},
  ]) {
    final request = jsonEncode({
      'profiles': ids,
      'primaryPasswords': primaryPasswords,
    });
    return _background(
      (c, a) => c._with([
        a,
        request,
      ], (p) => c._answer(c._browserImport(p[0], p[1]))),
    );
  }

  /// Turns off the chosen profiles' own password saving: `profiles`, each
  /// with its `result`.
  static Future<Answer> browserTurnOff(List<String> ids) {
    final list = jsonEncode(ids);
    return _background(
      (c, a) => c._with([
        a,
        list,
      ], (p) => c._answer(c._browserTurnOff(p[0], p[1]))),
    );
  }

  /// Made-up browsers for [email] in [root], laid out for [platform]
  /// ('windows', 'mac', 'linux', or '' for this computer's), which the core
  /// then looks at instead of none: for tests, with a core built with its
  /// test hooks (which never looks at the real ones). An empty [root] takes
  /// them away. Answers the made-up Firefox profile's Primary Password.
  @visibleForTesting
  String testBrowsers(String root, {String platform = '', String email = ''}) =>
      (_with([root, platform, email], (p) => _answer(_testBrowsers(p[0], p[1], p[2])))
              as Map<String, dynamic>)['primaryPassword']
          as String;

  /// The browser profiles read since this was last asked, by id: tests.
  @visibleForTesting
  List<String> testBrowsersRead() =>
      ((_answer(_testBrowsersRead()) as Map<String, dynamic>)['read'] as List)
          .cast<String>();

  /// Copies what the generator shows: the core takes it as the generator's
  /// last value, then copies that as it copies any secret, which clears in
  /// 30 seconds (on a phone, through the phone's own code).
  Future<Answer> copyGenerated(String shown) async {
    final kept = Answer(
      _with([account, shown], (p) => _answer(_keepGenerated(p[0], p[1])))
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

  /// Whether PassBundle's windows may be in screenshots, screen recordings,
  /// and screen sharing on this computer (window_chrome.dart): false until
  /// the person allows them.
  bool screenshotsAllowed() =>
      (_answer(_screenshots()) as Map<String, dynamic>)['allow'] == true;

  Answer setScreenshotsAllowed(bool allow) => Answer(
    _with([
          jsonEncode({'allow': allow}),
        ], (p) => _answer(_setScreenshots(p[0])))
        as Map<String, dynamic>,
  );

  /// The logins with something to fix (core/src/health.rs): `items`, each
  /// `{id, weak, reused, unsecured}`. Judged on a background isolate.
  static Future<Answer> health() =>
      _background((c, a) => c._with([a], (p) => c._answer(c._health(p[0]))));

  /// The expiration reminders due now (core/src/expiration.rs), each said
  /// once: `title` ("2 items expire soon", or null when nothing is due),
  /// `due` and `standing` (`{soon, expired}`). On a computer the core also
  /// tells the desktop agent's tray `standing`: how many items, never which.
  static Future<Answer> reminders() => _background(
    (c, a) => c._with([a], (p) => c._answer(c._reminders(p[0]))),
  );

  /// An item's past versions (core/src/history.rs), from the server:
  /// `{hidePasswords, canRestore, canClear, unreadable, versions: [{id,
  /// revisionDate, savedBy, opens, restorable, changes, rows}]}`.
  static Future<Answer> history(String id) => _background(
    (c, a) => c._with([a, id], (p) => c._answer(c._history(p[0], p[1]))),
  );

  /// One version beside the item now (`rows`, each `{label, version,
  /// current, secret, changed}`), its secrets shown when [reveal].
  Answer itemVersion(String id, String versionId, {bool reveal = false}) =>
      Answer(
        _with(
          [account, id, versionId, '$reveal'],
          (p) => _answer(_itemVersion(p[0], p[1], p[2], p[3])),
        ) as Map<String, dynamic>,
      );

  /// Restores a version, as a normal edit, then syncs.
  static Future<Answer> restoreVersion(String id, String versionId) =>
      _background(
        (c, a) => c._with([
          a,
          id,
          versionId,
        ], (p) => c._answer(c._restoreVersion(p[0], p[1], p[2]))),
      );

  /// Clears an item's history (no version), or deletes one version.
  static Future<Answer> deleteHistory(String id, [String? versionId]) =>
      _background(
        (c, a) => c._with([
          a,
          id,
          versionId ?? '',
        ], (p) => c._answer(c._deleteHistory(p[0], p[1], p[2]))),
      );

  /// Favorites an item or not, then syncs.
  static Future<Answer> setFavorite(String id, bool favorite) => _background(
    (c, a) => c._with([
      a,
      id,
      '$favorite',
    ], (p) => c._answer(c._setFavorite(p[0], p[1], p[2]))),
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
      (_with([account], (p) => _answer(_templates(p[0]))) as List)
          .cast<Map<String, dynamic>>();

  List<Map<String, dynamic>> list(String query) =>
      (_with([account, query], (p) => _answer(_list(p[0], p[1]))) as List)
          .cast<Map<String, dynamic>>();

  Map<String, dynamic>? item(String id) =>
      _with([account, id], (p) => _answer(_item(p[0], p[1])))
          as Map<String, dynamic>?;

  /// A secret, while the person reveals it or edits: `password`, `notes`, `totp`, or `field:<index>`.
  String? reveal(String id, String what) =>
      _with([account, id, what], (p) => _answer(_reveal(p[0], p[1], p[2])))
          as String?;

  /// Copies without the value passing through Dart; secrets clear after 30
  /// seconds. On phones the app's own code (Kotlin, Swift) takes it from the
  /// core to the clipboard.
  Future<Answer> copy(String id, String what) async {
    if (isPhone) {
      final copied = await platform.invokeMethod<bool>('copy', {
        'account': account,
        'id': id,
        'what': what,
      });
      return Answer({
        'ok': copied == true,
        if (copied != true) 'error': 'Couldn\'t copy that. Try again.',
      });
    }
    return Answer(
      _with([account, id, what], (p) => _answer(_copy(p[0], p[1], p[2])))
          as Map<String, dynamic>,
    );
  }

  Map<String, dynamic>? totp(String id) =>
      _with([account, id], (p) => _answer(_totp(p[0], p[1])))
          as Map<String, dynamic>?;

  /// The generator's last choices on this device (core/src/generator.rs):
  /// `kind` (`password` or `passphrase`), `length`, `uppercase`,
  /// `lowercase`, `digits`, `symbols`, `avoidAmbiguous`, `words`,
  /// `separator`, `capitalize`, and `includeNumber`.
  Map<String, dynamic> generatorOptions() =>
      _with([account], (p) => _answer(_generatorOptions(p[0])))
          as Map<String, dynamic>;

  /// The organizations' "Password generator" policy: `kind` (the only kind
  /// allowed, or null), `minLength`, `uppercase`, `lowercase`, `digits`,
  /// `symbols`, `minDigits`, `minSymbols`, `minWords`, `capitalize`, and
  /// `includeNumber`.
  Map<String, dynamic> generatorPolicy() =>
      _with([account], (p) => _answer(_generatorPolicy(p[0])))
          as Map<String, dynamic>;

  /// A new password or passphrase for the editor's field: `{ok, password,
  /// options}`, with the choices held to the organizations' policy, or the
  /// words to show. Without options, the last choices; the choices become
  /// the next ones.
  Answer generate([Map<String, dynamic>? options]) => Answer(
    _with([
      account,
      options == null ? 'null' : jsonEncode(options),
    ], (p) => _answer(_generate(p[0], p[1]))) as Map<String, dynamic>,
  );
}
