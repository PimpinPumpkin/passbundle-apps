// Shared by the app's tests: the real fonts (tests otherwise draw boxes), the
// core opened on a folder of the test's own, waiting for slow calls, which
// run on real isolates outside the test's fake clock, and a phone stood in
// for on this computer.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';

/// The sample vault's master password (the core's test vectors).
const samplePassword = 'correct horse battery staple 1';

Future<void> loadFonts() async {
  await (FontLoader(
    'Google Sans Flex',
  )..addFont(rootBundle.load('assets/fonts/GoogleSansFlex-wght.ttf'))).load();
  await (FontLoader(
    'MaterialIcons',
  )..addFont(rootBundle.load('fonts/MaterialIcons-Regular.otf'))).load();
}

/// The core on a new, empty folder: signed out.
String openEmpty({bool lockOnSleep = false}) {
  final dir = Directory.systemTemp.createTempSync('pb-app-test-').path;
  Core().openAt(dir);
  machineDoesntLock(lockOnSleep);
  return dir;
}

/// The core on a new folder holding the sample vault (test/sample): locked.
/// It's the core's test vectors' vault, with what PassBundle's server says
/// with each item and shared folder (`edit`, `viewPassword`, `permissions`,
/// `readOnly`, `hidePasswords`, `manage`): the person may do anything.
/// With `policies`, its kept sync has them, as the server sends them; the
/// items in `asking` ask for the master password again, as the web vault
/// sets it; and `changed` items (by id) and shared folders have those of
/// the server's plain fields (`edit`, `viewPassword`, `permissions`, a
/// shared folder's `keykeeper` grid) set as given.
String openSample({
  bool lockOnSleep = false,
  List<Map<String, dynamic>>? policies,
  Set<String> asking = const {},
  Map<String, Map<String, dynamic>> changed = const {},
}) {
  final dir = Directory.systemTemp.createTempSync('pb-app-test-').path;
  for (final name in ['settings.json', 'sync.json']) {
    File('test/sample/$name').copySync('$dir/$name');
  }
  if (policies != null || asking.isNotEmpty || changed.isNotEmpty) {
    final sync = File('$dir/sync.json');
    final json = jsonDecode(sync.readAsStringSync()) as Map<String, dynamic>;
    for (final cipher
        in (json['ciphers'] as List).cast<Map<String, dynamic>>()) {
      if (asking.contains(cipher['id'])) cipher['reprompt'] = 1;
      cipher.addAll(changed[cipher['id']] ?? const {});
    }
    for (final collection
        in (json['collections'] as List).cast<Map<String, dynamic>>()) {
      collection.addAll(changed[collection['id']] ?? const {});
    }
    sync.writeAsStringSync(
      jsonEncode({...json, 'policies': policies ?? json['policies']}),
    );
  }
  Core().openAt(dir);
  machineDoesntLock(lockOnSleep);
  return dir;
}

/// For screenshots: the fuller sample vault core/examples/showcase.rs
/// writes (folders in folders, shared folders, templates), from the folder
/// PB_SHOWCASE names; the sample vault without it. The same account and
/// master password either way.
String openShowcase() {
  final from = Platform.environment['PB_SHOWCASE'];
  if (from == null) return openSample();
  final dir = Directory.systemTemp.createTempSync('pb-app-test-').path;
  for (final name in ['settings.json', 'sync.json']) {
    File('$from/$name').copySync('$dir/$name');
  }
  Core().openAt(dir);
  machineDoesntLock(false);
  return dir;
}

/// The core reads the real computer's screen: on a test machine whose screen
/// is locked (no one at it), the vault would lock in the middle of a test. So
/// tests start with "Lock when the computer sleeps" off, unless they're about it.
void machineDoesntLock(bool lockOnSleep) {
  final answer = Core().setPreferences({
    'lockAfterMinutes': 15,
    'lockOnSleep': lockOnSleep,
  });
  if (!answer.ok) throw StateError('${answer.error}');
}

/// Pumps until something shows up, giving slow core calls real time.
Future<void> pumpUntil(
  WidgetTester tester,
  Finder finder, {
  Duration timeout = const Duration(seconds: 20),
}) async {
  final end = DateTime.now().add(timeout);
  while (DateTime.now().isBefore(end)) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
    await tester.pump();
    if (finder.evaluate().isNotEmpty) return;
  }
  throw TestFailure('Never showed up: $finder');
}

/// A phone, on this computer: the phone's words and screens, with a stand-in
/// for its own code (MainActivity.kt, AppDelegate.swift) that answers about
/// biometrics as a phone with a fingerprint set up would. `enrolled` counts
/// the times turning it on was asked for; each asks for the fingerprint, and
/// the stand-in gives it unless [biometricAnswers] says no.
class PhoneStandIn {
  PhoneStandIn(
    WidgetTester tester, {
    this.status = 'notSetUp',
    this.method = 'fingerprint',
  }) {
    debugPhone = true;
    addTearDown(() => debugPhone = null);
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(Core.platform, (call) async {
      switch (call.method) {
        case 'biometricStatus':
          return {'status': status, 'method': method};
        case 'biometricEnroll':
          enrolled++;
          final yes = biometricAnswers.isEmpty || biometricAnswers.removeAt(0);
          if (yes) status = 'setUp';
          return {'ok': true, 'on': yes};
        case 'biometricRemove':
          status = 'notSetUp';
          return {'ok': true};
        default:
          return null;
      }
    });
    addTearDown(() => messenger.setMockMethodCallHandler(Core.platform, null));
  }

  /// "setUp", "notSetUp", or "unavailable", as Biometric.status() says.
  String status;
  final String method;
  var enrolled = 0;

  /// What the person does at each biometric prompt: true gives it.
  final biometricAnswers = <bool>[];
}
