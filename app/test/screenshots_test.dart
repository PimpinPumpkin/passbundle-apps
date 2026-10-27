// Screenshots for people to look at, not checks: run with --update-goldens
// to (re)draw them in test/screenshots/. Desktop and phone width, light and
// dark; the phone's at 390 wide, and Settings also at 360 with the text 1.3
// times its size, as a phone set to large text draws it. With PB_SHOWCASE
// (core/examples/showcase.rs), on a fuller vault: folders in folders,
// shared folders, and items made from templates. The computer's are drawn
// with the window's top as Windows has it (window_chrome.dart), and an item
// also as a Mac (with stand-ins for its traffic lights) and Linux have it.
@Tags(['screenshots'])
library;

import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/icons.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/sign_in_requests.dart';
import 'package:passbundle/updates.dart';
import 'package:passbundle/window_chrome.dart';

import 'support.dart';

Future<void> size(WidgetTester tester, Size size, {double text = 1}) async {
  tester.view.physicalSize = size * 2;
  tester.view.devicePixelRatio = 2;
  tester.platformDispatcher.textScaleFactorTestValue = text;
  addTearDown(tester.view.reset);
  addTearDown(tester.platformDispatcher.clearAllTestValues);
}

/// Draws what's on screen: the websites' icons first, which load outside
/// the test's clock.
Future<void> shoot(String name, [WidgetTester? tester, Finder? what]) async {
  if (tester != null) await tester.pump();
  await expectLater(
    what ?? find.byType(MaterialApp),
    matchesGoldenFile('screenshots/$name.png'),
  );
}

const windows = WindowFrame(
  style: FrameStyle.windows,
  end: WindowButton.values,
);

/// A computer's window, as its runner would describe it (window_chrome.dart):
/// [lights] hears where a Mac's traffic lights would go.
void onComputer(
  WidgetTester tester,
  WindowFrame frame, {
  ValueNotifier<List<double>?>? lights,
}) {
  final messenger = tester.binding.defaultBinaryMessenger;
  messenger.setMockMethodCallHandler(WindowFrameKeeper.channel, (call) async {
    if (call.method == 'chrome' && lights != null) {
      lights.value = ((call.arguments as Map)['lights'] as List?)
          ?.cast<double>();
    }
    return null;
  });
  WindowFrameKeeper.instance.debugFrame = frame;
  addTearDown(() {
    messenger.setMockMethodCallHandler(WindowFrameKeeper.channel, null);
    WindowFrameKeeper.instance.debugFrame = const WindowFrame();
  });
}

/// The app, with a Mac's traffic lights where it has the runner put them:
/// the real ones are AppKit's, which a test hasn't.
class _WithLights extends StatelessWidget {
  const _WithLights({required this.lights, required this.child});

  final ValueListenable<List<double>?> lights;
  final Widget child;

  @override
  Widget build(BuildContext context) => Directionality(
    textDirection: TextDirection.ltr,
    child: Stack(
      children: [
        child,
        ValueListenableBuilder(
          valueListenable: lights,
          builder: (context, at, _) => at == null
              ? const SizedBox.shrink()
              : Positioned(
                  // 12 across, 20 apart, as macOS draws them.
                  left: at[0] + 1,
                  top: at[1] - 6,
                  child: Row(
                    children: [
                      for (final color in const [
                        Color(0xFFFF5F57),
                        Color(0xFFFEBC2E),
                        Color(0xFF28C840),
                      ])
                        Container(
                          width: 12,
                          height: 12,
                          margin: const EdgeInsets.only(right: 8),
                          decoration: BoxDecoration(
                            color: color,
                            shape: BoxShape.circle,
                          ),
                        ),
                    ],
                  ),
                ),
        ),
      ],
    ),
  );
}

Future<void> unlock(WidgetTester tester, {bool phone = false}) async {
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  // A phone's vault starts with its kinds; a computer's lists the items.
  await pumpUntil(tester, find.text(phone ? 'Logins' : 'Backup service'));
  // The offline banner (no server behind the sample) settles first.
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
}

/// A sign-in waiting for an answer (sign_in_requests.dart), as the core
/// would list it, so the prompt shows over the vault.
class _SignInRequests implements SignInRequestSource {
  @override
  Future<Answer> list() async => Answer({
    'ok': true,
    'requests': [
      {
        'id': 'shot',
        'device': 'Web vault in Chrome',
        'ip': '203.0.113.7',
        'fingerprint': 'caretaker-uncaring-constrain-rind-dove',
        'ageSeconds': 20,
        'secondsLeft': 880,
      },
    ],
  });

  @override
  Future<Answer> answer(String id, String fingerprint, bool approve) async =>
      Answer({'ok': true});

  @override
  Future<bool> confirm(BuildContext context) async => true;

  @override
  bool confirmApprovals = true;
}

/// Android's updates, as Settings shows them there: this version, checked.
class _Updates implements UpdatePlatform {
  @override
  Future<Map<String, dynamic>> status() async => {
    'version': '0.1.4',
    'store': null,
    'auto': true,
  };

  @override
  Future<Map<String, dynamic>> check({required bool quiet}) async => {
    'ok': true,
    'available': false,
  };

  @override
  Future<Map<String, dynamic>> install() async => {'ok': true};

  @override
  Future<void> setAuto(bool on) async {}

  @override
  set onEvent(void Function(String, Map<String, dynamic>) handler) {}

  // Anything else the native code does for updates, which these don't show.
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

/// The switch for unlocking with Windows Hello or Touch ID, in Settings.
const biometric = ValueKey('biometric');

/// The showcase's websites' icons, drawn ahead (outside the test's clock,
/// where images draw), so tiles show them rather than waiting.
Future<void> precacheIcons(WidgetTester tester) => tester.runAsync(
  () => Future.wait([for (final host in showcaseIcons) siteIcons.of(host)]),
);

/// A screenshot test, with shadows drawn as a phone or computer draws them:
/// tests otherwise draw them as solid blocks, which keeps checks steady, but
/// these are only for looking at.
void shots(
  String description,
  Future<void> Function(WidgetTester tester) body, {
  bool? skip,
}) => testWidgets(description, (tester) async {
  debugDisableShadows = false;
  try {
    await body(tester);
  } finally {
    debugDisableShadows = true;
  }
}, skip: skip);

void main() {
  setUpAll(loadFonts);

  for (final dark in [false, true]) {
    final b = dark ? 'dark' : 'light';
    final theme = dark ? ThemeMode.dark : ThemeMode.light;

    shots('sign in, $b', (tester) async {
      await size(tester, const Size(1280, 800));
      onComputer(tester, windows);
      openEmpty();
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await shoot('sign-in-$b', tester);
    });

    shots('unlock, vault, item, editor, desktop, $b', (tester) async {
      await size(tester, const Size(1280, 800));
      onComputer(tester, windows);
      openShowcase();
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await precacheIcons(tester);
      await shoot('unlock-$b', tester);
      await unlock(tester);
      // The menu, the list, and the item's place beside it, still empty.
      await shoot('vault-desktop-$b', tester);
      await tester.tap(find.text('Example Mail').first);
      await tester.pump();
      await tester.tap(find.byTooltip('Reveal').first);
      await tester.pump();
      await shoot('item-desktop-$b', tester);
      await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
      await tester.pumpAndSettle();
      await shoot('editor-desktop-$b', tester);
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();

      if (Platform.environment['PB_SHOWCASE'] != null) {
        Future<void> open(String search, String name) async {
          await tester.enterText(find.byType(TextField).first, search);
          await tester.pump();
          await tester.tap(find.text(name).first);
          await tester.pump();
        }

        // An item made from a template, in an organization's shared folder.
        await open('office', 'Office server');
        await shoot('template-item-desktop-$b', tester);
        // One in a shared folder that hides its passwords.
        await open('vpn', 'Fabrikam VPN');
        await shoot('hidden-passwords-desktop-$b', tester);
        await tester.enterText(find.byType(TextField).first, '');
        await tester.pump();
      }

      // New item: the kinds, then More categories' templates.
      await tester.tap(find.text('New'));
      await tester.pumpAndSettle();
      await shoot('new-item-desktop-$b', tester);
      await tester.tap(find.text('More categories'));
      await tester.pumpAndSettle();
      await shoot('new-item-more-desktop-$b', tester);
      await tester.tap(find.text('Wi-Fi network'));
      await tester.pumpAndSettle();
      await shoot('editor-template-desktop-$b', tester);
    });

    // On a real server (PB_TEST_SERVER, tools/test-server.sh): an item's
    // versions, and one beside the item now.
    shots('version history, desktop, $b', (tester) async {
      final server = Platform.environment['PB_TEST_SERVER']!;
      await size(tester, const Size(1280, 800));
      onComputer(tester, windows);
      openEmpty();
      final email =
          'history-${DateTime.now().microsecondsSinceEpoch}@example.test';
      final made = await tester.runAsync(
        () => Core.signUp(
          server,
          email,
          '',
          'correct horse battery staple 9',
          '',
        ),
      );
      expect(made!.ok, isTrue, reason: made.error);
      Future<void> save(
        String? id,
        String name,
        String password,
        String port,
      ) async {
        final saved = await tester.runAsync(
          () => Core.save(id, {
            'name': name,
            'username': 'alex@contoso.example',
            'password': password,
            'uris': ['https://mail.contoso.example'],
            'fields': [
              {'name': 'Port', 'value': port, 'type': 0},
            ],
            'collectionIds': <String>[],
          }),
        );
        expect(saved!.ok, isTrue, reason: saved.error);
      }

      await save(null, 'Contoso Mail', 'not-a-real-password-1', '993');
      final id = Core().list('').single['id'] as String;
      await save(id, 'Contoso Mail', 'not-a-real-password-2', '993');
      await save(id, 'Contoso Mail (work)', 'not-a-real-password-3', '995');
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await pumpUntil(tester, find.text('Contoso Mail (work)'));
      await tester.tap(find.text('Contoso Mail (work)').first);
      await tester.pump();
      await tester.ensureVisible(find.text('Version history'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Version history'));
      await pumpUntil(tester, find.textContaining('Saved by you'));
      await tester.pump(const Duration(milliseconds: 400));
      await shoot('history-desktop-$b', tester);
      await tester.tap(find.textContaining('Saved by you').last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Show hidden values'));
      await tester.pumpAndSettle();
      await shoot('version-desktop-$b', tester);
    }, skip: Platform.environment['PB_TEST_SERVER'] == null);

    shots('unlocking with Windows Hello or Touch ID, $b', (tester) async {
      await size(tester, const Size(1280, 800));
      onComputer(tester, windows);
      final program = Platform.environment['PB_TEST_AGENT']!;
      final home = Directory('/tmp').createTempSync('pb-ag-').path;
      final agent = (await tester.runAsync(
        () => Process.start(
          program,
          ['--serve'],
          environment: {
            'HOME': home,
            'PASSBUNDLE_AGENT_TEST_HOME': home,
            'PASSBUNDLE_AGENT_TEST_AUTHENTICATOR': home,
          },
        ),
      ))!;
      addTearDown(agent.kill);
      openSample();
      Core().useAgentHome(home, program);
      for (var i = 0; i < 100; i++) {
        if ((await tester.runAsync(Core.biometricStatus))!['status'] !=
            'noAgent') {
          break;
        }
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 100)),
        );
      }
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await unlock(tester);
      await tester.tap(find.byTooltip('Settings'));
      await tester.pump();
      await pumpUntil(tester, find.byKey(biometric));
      File('$home/answers').writeAsStringSync('yes');
      await tester.tap(find.byKey(biometric));
      await pumpUntil(
        tester,
        find.byWidgetPredicate(
          (w) => w is SwitchListTile && w.key == biometric && w.value,
        ),
      );
      // Past the dialog's fade-in, which the test's clock doesn't advance by itself.
      await tester.pump(const Duration(milliseconds: 400));
      await shoot('settings-$b', tester);
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Lock'));
      await tester.pump();
      await pumpUntil(tester, find.textContaining('Unlock with '));
      await shoot('unlock-with-biometrics-$b', tester);
    }, skip: Platform.environment['PB_TEST_AGENT'] == null);

    shots('an item, with the window\'s top as a Mac and Linux have it, $b', (
      tester,
    ) async {
      await size(tester, const Size(1280, 800));
      final lights = ValueNotifier<List<double>?>(null);
      onComputer(
        tester,
        const WindowFrame(style: FrameStyle.mac),
        lights: lights,
      );
      openShowcase();
      await tester.pumpWidget(
        _WithLights(
          lights: lights,
          child: PassBundleApp(themeMode: theme),
        ),
      );
      await precacheIcons(tester);
      await unlock(tester);
      await tester.tap(find.text('Example Mail').first);
      await tester.pump();
      await tester.pump();
      await shoot('item-desktop-mac-$b', tester, find.byType(_WithLights));
      // Ubuntu's: minimize, maximize, and close, at the end.
      WindowFrameKeeper.instance.debugFrame = const WindowFrame(
        style: FrameStyle.linux,
        end: [WindowButton.minimize, WindowButton.maximize, WindowButton.close],
      );
      await tester.pump();
      await tester.pump();
      await shoot('item-desktop-linux-$b', tester, find.byType(_WithLights));
    });

    shots('a sign-in to approve, desktop and phone, $b', (tester) async {
      signInRequests.source = _SignInRequests();
      addTearDown(() => signInRequests.source = CoreSignInRequests());
      await size(tester, const Size(1280, 800));
      onComputer(tester, windows);
      openShowcase();
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await precacheIcons(tester);
      await unlock(tester);
      await pumpUntil(tester, find.text('Approve sign-in?'));
      await tester.pump(const Duration(milliseconds: 400));
      await shoot('sign-in-request-desktop-$b', tester);
      await tester.tap(find.text('Later'));
      await tester.pumpAndSettle();
    });

    shots('a sign-in to approve, phone, $b', (tester) async {
      signInRequests.source = _SignInRequests();
      addTearDown(() => signInRequests.source = CoreSignInRequests());
      await size(tester, const Size(390, 844));
      PhoneStandIn(tester);
      openShowcase();
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await precacheIcons(tester);
      await unlock(tester, phone: true);
      // After the offer to unlock with a fingerprint.
      await pumpUntil(tester, find.text('Unlock with fingerprint next time?'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Not now'));
      await tester.pumpAndSettle();
      await pumpUntil(tester, find.text('Approve sign-in?'));
      await tester.pump(const Duration(milliseconds: 400));
      await shoot('sign-in-request-phone-$b', tester);
      await tester.tap(find.text('Later'));
      await tester.pumpAndSettle();
    });

    shots('vault, item, editor, and the offer, phone, $b', (tester) async {
      await size(tester, const Size(390, 844));
      PhoneStandIn(tester);
      openShowcase();
      await tester.pumpWidget(PassBundleApp(themeMode: theme));
      await precacheIcons(tester);
      await unlock(tester, phone: true);
      // Unlocked with the master password, on a phone with a fingerprint.
      await pumpUntil(tester, find.text('Unlock with fingerprint next time?'));
      await tester.pumpAndSettle();
      await shoot('unlock-offer-phone-$b', tester);
      await tester.tap(find.text('Not now'));
      await tester.pumpAndSettle();
      await shoot('vault-phone-$b', tester);
      // Its folders: Personal's, then each organization's shared folders.
      await tester.scrollUntilVisible(
        find.text('Folders'),
        200,
        scrollable: find.byType(Scrollable).last,
      );
      await tester.drag(find.byType(Scrollable).last, const Offset(0, -260));
      await tester.pumpAndSettle();
      await shoot('folders-phone-$b', tester);
      await tester.drag(find.byType(Scrollable).last, const Offset(0, 4000));
      await tester.pumpAndSettle();

      await tester.enterText(find.byType(TextField), 'backup');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Backup service'));
      await tester.pumpAndSettle();
      await shoot('item-phone-$b', tester);
      await tester.pageBack();
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('Generator'));
      await tester.pumpAndSettle();
      await shoot('generator-phone-$b', tester);
      await tester.tap(find.byTooltip('Items'));
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('New item'));
      await tester.pumpAndSettle();
      await shoot('new-item-phone-$b', tester);
      await tester.tap(find.text('Login'));
      await tester.pumpAndSettle();
      await shoot('editor-phone-$b', tester);
    });

    for (final (width, text) in [(390.0, 1.0), (360.0, 1.3)]) {
      final name = text == 1
          ? 'settings-phone-$b'
          : 'settings-phone-${width.toInt()}-large-text-$b';
      shots(name, (tester) async {
        await size(tester, Size(width, 844), text: text);
        PhoneStandIn(tester, status: 'setUp');
        updates = Updates(_Updates());
        addTearDown(() => updates = null);
        openSample();
        await tester.pumpWidget(PassBundleApp(themeMode: theme));
        await unlock(tester, phone: true);
        await tester.tap(find.byTooltip('Settings'));
        await tester.pump();
        await pumpUntil(tester, find.byKey(biometric));
        await tester.pumpAndSettle();
        await shoot(name, tester);
      });
    }
  }
}
