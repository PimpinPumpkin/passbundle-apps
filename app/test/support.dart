// Shared by the app's tests: the real fonts (tests otherwise draw boxes), the
// core opened on a folder of the test's own, and waiting for slow calls,
// which run on real isolates outside the test's fake clock.
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';

/// The sample vault's master password (the core's test vectors).
const samplePassword = 'correct horse battery staple 1';

Future<void> loadFonts() async {
  final inter = FontLoader('Inter');
  for (final weight in ['Regular', 'Medium', 'SemiBold', 'Bold']) {
    inter.addFont(rootBundle.load('assets/fonts/Inter-$weight.ttf'));
  }
  await inter.load();
  await (FontLoader(
    'MaterialIcons',
  )..addFont(rootBundle.load('fonts/MaterialIcons-Regular.otf'))).load();
}

/// The core on a new, empty folder: signed out.
String openEmpty() {
  final dir = Directory.systemTemp.createTempSync('pb-app-test-').path;
  Core().openAt(dir);
  return dir;
}

/// The core on a new folder holding the sample vault (test/sample): locked.
String openSample() {
  final dir = Directory.systemTemp.createTempSync('pb-app-test-').path;
  for (final name in ['settings.json', 'sync.json']) {
    File('test/sample/$name').copySync('$dir/$name');
  }
  Core().openAt(dir);
  return dir;
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
