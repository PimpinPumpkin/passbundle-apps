// Settings' Updates and the notice over the vault (updates.dart), with a
// stand-in for the app's Kotlin, which does the checking and installing on
// Android (update/Updater.kt, tested on its own in app/android/app/src/test).
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/theme.dart';
import 'package:passbundle/updates.dart';

class StandIn implements UpdatePlatform {
  StandIn({
    this.store,
    this.checkAnswer = const {'ok': true, 'available': false},
  });

  final String? store;
  Map<String, dynamic> checkAnswer;
  Map<String, dynamic> installAnswer = {'ok': true};
  final checks = <bool>[];
  var installs = 0;
  bool? auto;
  void Function(String, Map<String, dynamic>)? event;

  @override
  Future<Map<String, dynamic>> status() async => {
    'version': '0.1.4',
    'store': store,
    'auto': true,
  };

  @override
  Future<Map<String, dynamic>> check({required bool quiet}) async {
    checks.add(quiet);
    return checkAnswer;
  }

  @override
  Future<Map<String, dynamic>> install() async {
    installs++;
    event?.call('progress', {'percent': 40});
    return installAnswer;
  }

  @override
  Future<void> setAuto(bool on) async => auto = on;

  @override
  set onEvent(void Function(String, Map<String, dynamic>) handler) =>
      event = handler;
}

const newer = {
  'ok': true,
  'available': true,
  'version': '0.1.5',
  'notes': '- Faster sync\n- Updates in Settings',
};

Future<void> showSection(WidgetTester tester, Updates u) async {
  await tester.pumpWidget(
    MaterialApp(
      theme: passBundleTheme(Brightness.light),
      home: Scaffold(
        body: SingleChildScrollView(child: UpdatesSection(updates: u)),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  tearDown(() => updates = null);

  testWidgets('Settings shows the version, checks, and offers what it found', (
    tester,
  ) async {
    final standIn = StandIn(checkAnswer: newer);
    final u = Updates(standIn);
    await showSection(tester, u);
    expect(find.text('Version 0.1.4'), findsOneWidget);
    expect(find.text('Update to 0.1.5'), findsNothing);

    await tester.tap(find.text('Check for updates'));
    await tester.pumpAndSettle();
    expect(standIn.checks, [false]);
    expect(find.text('PassBundle 0.1.5 is out.'), findsOneWidget);
    expect(find.text("What's new in 0.1.5"), findsOneWidget);
    expect(find.text('- Faster sync\n- Updates in Settings'), findsOneWidget);
    // Nothing downloads until the person taps it.
    expect(standIn.installs, 0);

    await tester.tap(find.text('Update to 0.1.5'));
    await tester.pumpAndSettle();
    expect(standIn.installs, 1);
    // Android's prompt takes it from here; the button is back for a cancel.
    expect(find.text('Update to 0.1.5'), findsOneWidget);
  });

  testWidgets('Settings says when it is the newest, and what went wrong', (
    tester,
  ) async {
    final standIn = StandIn();
    final u = Updates(standIn);
    await showSection(tester, u);
    await tester.tap(find.text('Check for updates'));
    await tester.pumpAndSettle();
    expect(find.text('This is the newest version.'), findsOneWidget);

    standIn.checkAnswer = {
      'ok': false,
      'error':
          "Couldn't check for updates. Check your connection and try again.",
    };
    await tester.tap(find.text('Check for updates'));
    await tester.pumpAndSettle();
    expect(
      find.text(
        "Couldn't check for updates. Check your connection and try again.",
      ),
      findsOneWidget,
    );
  });

  testWidgets('an update that fails to install says why', (tester) async {
    final standIn = StandIn(checkAnswer: newer)
      ..installAnswer = {
        'ok': false,
        'error':
            'The download didn\'t match the release, so it wasn\'t installed.',
      };
    final u = Updates(standIn);
    await showSection(tester, u);
    await tester.tap(find.text('Check for updates'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Update to 0.1.5'));
    await tester.pumpAndSettle();
    expect(
      find.text(
        'The download didn\'t match the release, so it wasn\'t installed.',
      ),
      findsOneWidget,
    );

    // Android's installer answers later, through the Kotlin.
    standIn.event!('installed', {
      'ok': false,
      'error': "The update wasn't installed.",
    });
    await tester.pumpAndSettle();
    expect(find.text("The update wasn't installed."), findsOneWidget);
  });

  testWidgets('installed by a store, it leaves updates to the store', (
    tester,
  ) async {
    final u = Updates(StandIn(store: 'F-Droid'));
    await showSection(tester, u);
    expect(find.text('Version 0.1.4'), findsOneWidget);
    expect(find.text('Updates come from F-Droid.'), findsOneWidget);
    expect(find.text('Check for updates'), findsNothing);
    expect(find.text('Check once a day'), findsNothing);
  });

  testWidgets('the daily check can be turned off', (tester) async {
    final standIn = StandIn();
    final u = Updates(standIn);
    await showSection(tester, u);
    await tester.tap(find.text('Check once a day'));
    await tester.pumpAndSettle();
    expect(standIn.auto, false);
    expect(u.auto, false);
  });

  testWidgets(
    'the check on start only shows a notice, once, which opens the update',
    (tester) async {
      final standIn = StandIn(checkAnswer: newer);
      updates = Updates(standIn);
      await tester.pumpWidget(
        MaterialApp(
          theme: passBundleTheme(Brightness.light),
          home: Scaffold(
            body: Builder(
              builder: (context) => TextButton(
                onPressed: () => showUpdateNotice(context),
                child: const Text('Vault'),
              ),
            ),
          ),
        ),
      );
      await checkForUpdatesOnStart();
      expect(standIn.checks, [true]);

      await tester.tap(find.text('Vault'));
      await tester.pumpAndSettle();
      expect(find.text('PassBundle 0.1.5 is out.'), findsOneWidget);
      expect(standIn.installs, 0);

      await tester.tap(find.text('See update'));
      await tester.pumpAndSettle();
      expect(find.text('Update PassBundle'), findsOneWidget);
      expect(find.text('Update to 0.1.5'), findsOneWidget);
      expect(standIn.installs, 0);
      await tester.tap(find.text('Not now'));
      await tester.pumpAndSettle();

      // Once: the vault coming back doesn't say it again.
      ScaffoldMessenger.of(tester.element(find.text('Vault')))
          .removeCurrentSnackBar();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Vault'));
      await tester.pumpAndSettle();
      expect(find.text('PassBundle 0.1.5 is out.'), findsNothing);
    },
  );

  testWidgets('a manual check never shows the notice', (tester) async {
    final standIn = StandIn(checkAnswer: newer);
    updates = Updates(standIn);
    await updates!.check();
    expect(updates!.takeNotice(), isNull);
  });
}
