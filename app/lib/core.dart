// The core (Rust), through its C bridge (core/src/ffi.rs). No key crosses
// here, and a secret only when the person reveals it or edits it
// (docs/core.md, "Memory"). Slow calls (signing in, unlocking, syncing,
// saving) run on a background isolate; the screens don't read while one runs.
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:isolate';

import 'package:ffi/ffi.dart';

typedef _Str0 = Pointer<Utf8> Function();
typedef _Str1 = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _Str2 = Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>);
typedef _Str3 = Pointer<Utf8> Function(
  Pointer<Utf8>,
  Pointer<Utf8>,
  Pointer<Utf8>,
);
typedef _FinishNative = Pointer<Utf8> Function(Uint8, Pointer<Utf8>, Bool);
typedef _Finish = Pointer<Utf8> Function(int, Pointer<Utf8>, bool);

DynamicLibrary _open() {
  // Tests and tools point at a library they built.
  final override = Platform.environment['PB_CORE_LIB'];
  if (override != null) return DynamicLibrary.open(override);
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
  return DynamicLibrary.process();
}

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
  late final _signIn = _lib.lookupFunction<_Str3, _Str3>('pb_sign_in');
  late final _finish = _lib.lookupFunction<_FinishNative, _Finish>(
    'pb_finish_sign_in',
  );
  late final _sendEmail = _lib.lookupFunction<_Str0, _Str0>(
    'pb_send_email_code',
  );
  late final _unlock = _lib.lookupFunction<_Str1, _Str1>('pb_unlock');
  late final _sync = _lib.lookupFunction<_Str0, _Str0>('pb_sync');
  late final _lock = _lib.lookupFunction<_Str0, _Str0>('pb_lock');
  late final _signOut = _lib.lookupFunction<_Str0, _Str0>('pb_sign_out');
  late final _preferences = _lib.lookupFunction<_Str0, _Str0>('pb_preferences');
  late final _setPreferences = _lib.lookupFunction<_Str1, _Str1>(
    'pb_set_preferences',
  );
  late final _used = _lib.lookupFunction<_Str0, _Str0>('pb_used');
  late final _tick = _lib.lookupFunction<_Str0, _Str0>('pb_tick');
  late final _sleeping = _lib.lookupFunction<_Str0, _Str0>('pb_sleeping');
  late final _places = _lib.lookupFunction<_Str0, _Str0>('pb_places');
  late final _list = _lib.lookupFunction<_Str1, _Str1>('pb_list');
  late final _item = _lib.lookupFunction<_Str1, _Str1>('pb_item');
  late final _reveal = _lib.lookupFunction<_Str2, _Str2>('pb_reveal');
  late final _copy = _lib.lookupFunction<_Str2, _Str2>('pb_copy');
  late final _totp = _lib.lookupFunction<_Str1, _Str1>('pb_totp');
  late final _save = _lib.lookupFunction<_Str2, _Str2>('pb_save');
  late final _trash = _lib.lookupFunction<_Str1, _Str1>('pb_trash');
  late final _generate = _lib.lookupFunction<_Str1, _Str1>('pb_generate');
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

  /// For tests and tools: the app on a folder of its own, the refresh token in memory.
  Map<String, dynamic> openAt(String dir) =>
      _with([dir], (p) => _answer(_openAt(p[0], true))) as Map<String, dynamic>;

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

  static Future<Answer> sync() => _background((c) => c._answer(c._sync()));

  static Future<Answer> save(String? id, Map<String, dynamic> draft) =>
      _background(
        (c) => c._with([
          id ?? '',
          jsonEncode(draft),
        ], (p) => c._answer(c._save(p[0], p[1]))),
      );

  /// Unlocking with Windows Hello or Touch ID, through the desktop agent:
  /// `status` is setUp, notSetUp, unavailable, or noAgent, and `method` its name.
  static Future<Map<String, dynamic>> biometricStatus() async =>
      Map<String, dynamic>.from(
        await Isolate.run(() {
          final core = Core();
          return core._answer(core._bioStatus()) as Map;
        }),
      );

  /// Starts the desktop agent installed next to the app if it isn't running
  /// (its tray, and Windows Hello). Windows and macOS, where the tray is.
  static Future<Answer> startAgent() =>
      _background((c) => c._answer(c._startAgent()));

  /// Turns it on (the agent asks); `on` says whether the person agreed.
  static Future<Answer> enrollBiometric() =>
      _background((c) => c._answer(c._bioEnroll()));

  /// Unlocks with it; `unlocked` is false if the person cancelled.
  static Future<Answer> unlockWithBiometric() =>
      _background((c) => c._answer(c._bioUnlock()));

  /// Turns it off, for the browser extension too.
  static Future<Answer> removeBiometric() =>
      _background((c) => c._answer(c._bioRemove()));

  /// For tests: the agent with this home folder and program, instead of the installed one.
  void useAgentHome(String home, String program) =>
      _with([home, program], (p) => _answer(_useAgentHome(p[0], p[1])));

  static Future<Answer> trash(String id) =>
      _background((c) => c._with([id], (p) => c._answer(c._trash(p[0]))));

  Map<String, dynamic> lock() => _answer(_lock()) as Map<String, dynamic>;

  Map<String, dynamic> signOut() => _answer(_signOut()) as Map<String, dynamic>;

  /// When to lock by itself: `lockAfterMinutes` (null: only when the app
  /// closes) and `lockOnSleep` (when the screen locks or the computer sleeps).
  Map<String, dynamic> preferences() =>
      _answer(_preferences()) as Map<String, dynamic>;

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
  Map<String, dynamic> sleeping() =>
      _answer(_sleeping()) as Map<String, dynamic>;

  /// Folders, and organizations with their collections.
  Map<String, dynamic> places() => _answer(_places()) as Map<String, dynamic>;

  List<Map<String, dynamic>> list(String query) =>
      (_with([query], (p) => _answer(_list(p[0]))) as List)
          .cast<Map<String, dynamic>>();

  Map<String, dynamic>? item(String id) =>
      _with([id], (p) => _answer(_item(p[0]))) as Map<String, dynamic>?;

  /// A secret, while the person reveals it or edits: `password`, `notes`, `totp`, or `field:<index>`.
  String? reveal(String id, String what) =>
      _with([id, what], (p) => _answer(_reveal(p[0], p[1]))) as String?;

  /// Copies without the value passing through Dart; secrets clear after 30 seconds.
  Answer copy(String id, String what) => Answer(
    _with([id, what], (p) => _answer(_copy(p[0], p[1])))
        as Map<String, dynamic>,
  );

  Map<String, dynamic>? totp(String id) =>
      _with([id], (p) => _answer(_totp(p[0]))) as Map<String, dynamic>?;

  /// A new password, for the editor's field.
  String? generate({int length = 20}) {
    final answer = _with([
      jsonEncode({'length': length}),
    ], (p) => _answer(_generate(p[0]))) as Map<String, dynamic>;
    return answer['password'] as String?;
  }
}
