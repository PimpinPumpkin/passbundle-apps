// Shared by the app's tests: the real fonts (tests otherwise draw boxes), the
// core opened on a folder of the test's own, and waiting for slow calls,
// which run on real isolates outside the test's fake clock.
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
/// With `policies`, its kept sync has them, as the server sends them; the
/// items in `asking` ask for the master password again, as the web vault sets it.
String openSample({
  bool lockOnSleep = false,
  List<Map<String, dynamic>>? policies,
  Set<String> asking = const {},
}) {
  final dir = Directory.systemTemp.createTempSync('pb-app-test-').path;
  for (final name in ['settings.json', 'sync.json']) {
    File('test/sample/$name').copySync('$dir/$name');
  }
  if (policies != null || asking.isNotEmpty) {
    final sync = File('$dir/sync.json');
    final json = jsonDecode(sync.readAsStringSync()) as Map<String, dynamic>;
    for (final cipher
        in (json['ciphers'] as List).cast<Map<String, dynamic>>()) {
      if (asking.contains(cipher['id'])) cipher['reprompt'] = 1;
    }
    sync.writeAsStringSync(
      jsonEncode({...json, 'policies': policies ?? json['policies']}),
    );
  }
  Core().openAt(dir);
  machineDoesntLock(lockOnSleep);
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
