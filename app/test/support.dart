// Shared by the app's tests: the real fonts (tests otherwise draw boxes), and
// the core unlocked with the sample vault, off the fake test clock.
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';

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

Future<Map<String, dynamic>> unlockSample(
  WidgetTester tester, [
  String password = samplePassword,
]) async {
  final answer = await tester.runAsync(() async {
    final prelogin = await rootBundle.loadString('assets/sample/prelogin.json');
    final sync = await rootBundle.loadString('assets/sample/sync.json');
    return Core.unlock(sync, prelogin, password);
  });
  return answer!;
}
