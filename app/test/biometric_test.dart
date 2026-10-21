// Unlocking with Windows Hello or Touch ID through a real desktop agent, built
// with its stand-in authenticator (PB_TEST_AGENT, the agent's
// `cargo build --features test-authenticator`): turned on in Settings, then
// the lock screen's button unlocks. Without PB_TEST_AGENT, it's skipped.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';

import 'support.dart';

final agentProgram = Platform.environment['PB_TEST_AGENT'];

/// The switch for unlocking with Windows Hello or Touch ID, in Settings.
const biometric = ValueKey('biometric');

void main() {
  setUpAll(loadFonts);

  testWidgets('turned on in Settings, the lock screen unlocks with it', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1280, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    // A private agent: its own home folder (so its own socket), and the stand-in's answers.
    // Short, since Unix socket paths are limited to about 100 bytes.
    final home = (Platform.isWindows ? Directory.systemTemp : Directory('/tmp'))
        .createTempSync('pb-ag-')
        .path;
    final agent = (await tester.runAsync(
      () => Process.start(
        agentProgram!,
        ['--serve'],
        environment: {
          'HOME': home,
          'USERPROFILE': home,
          'LOCALAPPDATA': home,
          'PASSBUNDLE_AGENT_TEST_HOME': home,
          'PASSBUNDLE_AGENT_TEST_AUTHENTICATOR': home,
        },
      ),
    ))!;
    addTearDown(agent.kill);
    void answer(String answers) =>
        File('$home/answers').writeAsStringSync(answers);

    openSample();
    Core().useAgentHome(home, agentProgram!);
    // Until the agent listens.
    for (var i = 0; i < 100; i++) {
      final status = await tester.runAsync(Core.biometricStatus);
      if (status!['status'] != 'noAgent') break;
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 100)),
      );
    }

    await tester.pumpWidget(const PassBundleApp());
    await tester.enterText(find.byType(TextField), samplePassword);
    await tester.tap(find.text('Unlock'));
    await pumpUntil(tester, find.text('Backup service'));
    await pumpUntil(
      tester,
      find.textContaining('Sign in again to get changes'),
    );
    // Not turned on yet, so the lock screen had no button for it.

    await tester.tap(find.byTooltip('Settings'));
    await tester.pump();
    await pumpUntil(tester, find.byKey(biometric));
    expect(tester.widget<SwitchListTile>(find.byKey(biometric)).value, false);
    answer('yes');
    await tester.tap(find.byKey(biometric));
    await pumpUntil(
      tester,
      find.byWidgetPredicate(
        (w) => w is SwitchListTile && w.key == biometric && w.value,
      ),
    );
    expect(
      File('$home/prompts').readAsStringSync(),
      contains('in the PassBundle app'),
    );
    await tester.tap(find.text('Close'));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Lock'));
    await tester.pump();
    await pumpUntil(tester, find.textContaining('Unlock with '));
    answer('yes');
    await tester.tap(find.textContaining('Unlock with '));
    await pumpUntil(tester, find.text('Backup service'));
  }, skip: agentProgram == null);
}
