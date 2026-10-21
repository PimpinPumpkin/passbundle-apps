// The core (Rust), through the prototype's C bridge (core/src/ffi.rs). The
// real app uses flutter_rust_bridge. No key crosses here, and a secret only
// when the person reveals it (docs/core.md, "Memory").
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

class Core {
  Core() : _lib = _open();

  final DynamicLibrary _lib;

  late final _version = _lib.lookupFunction<_Str0, _Str0>('pb_version');
  late final _list = _lib.lookupFunction<_Str1, _Str1>('pb_list');
  late final _item = _lib.lookupFunction<_Str1, _Str1>('pb_item');
  late final _reveal = _lib.lookupFunction<_Str2, _Str2>('pb_reveal');
  late final _totp = _lib.lookupFunction<_Str1, _Str1>('pb_totp');
  late final _unlock = _lib.lookupFunction<_Str3, _Str3>('pb_unlock');
  late final _lock = _lib.lookupFunction<Void Function(), void Function()>(
    'pb_lock',
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
        // Wipe before freeing: a password passes through here.
        final bytes = p.cast<Uint8>().asTypedList(p.length);
        bytes.fillRange(0, bytes.length, 0);
        malloc.free(p);
      }
    }
  }

  String version() => _answer(_version()) as String;

  /// Unlocks off the screen's thread: the KDF takes a moment.
  static Future<Map<String, dynamic>> unlock(
    String sync,
    String prelogin,
    String password,
  ) => Isolate.run(() {
    final core = Core();
    return core._with(
      [sync, prelogin, password],
      (p) => core._answer(core._unlock(p[0], p[1], p[2])),
    ) as Map<String, dynamic>;
  });

  List<Map<String, dynamic>> list(String query) =>
      (_with([query], (p) => _answer(_list(p[0]))) as List)
          .cast<Map<String, dynamic>>();

  Map<String, dynamic>? item(String id) =>
      _with([id], (p) => _answer(_item(p[0]))) as Map<String, dynamic>?;

  /// A secret, while the person reveals it: `password`, `notes`, or `field:<index>`.
  String? reveal(String id, String what) =>
      _with([id, what], (p) => _answer(_reveal(p[0], p[1]))) as String?;

  Map<String, dynamic>? totp(String id) =>
      _with([id], (p) => _answer(_totp(p[0]))) as Map<String, dynamic>?;

  void lock() => _lock();
}
