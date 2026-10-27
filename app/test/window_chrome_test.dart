// The window's top on a computer (window_chrome.dart), as each runner
// describes its window: on a Mac, room in the menu's top for the traffic
// lights, lined up with the list's search, and none in full screen; on
// Windows, minimize, maximize, and close at the top right, beside the
// item's actions; on Linux, the buttons the desktop asks for, on its side.
// The runner hears where the band is and what in it takes clicks: every
// field, button, and menu there, and a dialog's barrier, so none of them
// moves the window instead. Screen readers and the keyboard reach the
// buttons. Phones, and a Linux desktop that keeps its own title bar, are
// as they were.
import 'dart:ui' show SemanticsAction;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/widgets.dart';
import 'package:passbundle/window_chrome.dart';

import 'support.dart';

/// What the app tells the runner, as it would a real one.
class RunnerStandIn {
  RunnerStandIn(WidgetTester tester, WindowFrame frame) {
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(WindowFrameKeeper.channel, (call) async {
      calls.add(call);
      return null;
    });
    WindowFrameKeeper.instance.debugFrame = frame;
    addTearDown(() {
      messenger.setMockMethodCallHandler(WindowFrameKeeper.channel, null);
      WindowFrameKeeper.instance.debugFrame = const WindowFrame();
    });
  }

  final calls = <MethodCall>[];

  /// The latest band and what in it takes clicks.
  Map<Object?, Object?> get chrome =>
      calls.lastWhere((c) => c.method == 'chrome').arguments
          as Map<Object?, Object?>;

  Rect _rect(Object? edges) {
    final e = (edges! as List).cast<double>();
    return Rect.fromLTRB(e[0], e[1], e[2], e[3]);
  }

  Rect get band => _rect(chrome['band']);
  List<Rect> get holes => [for (final h in chrome['holes']! as List) _rect(h)];
  Map<String, Rect> get buttons => {
    for (final MapEntry(:key, :value)
        in (chrome['buttons']! as Map<Object?, Object?>).entries)
      key! as String: _rect(value),
  };

  /// Whether a press at [point] would move the window, as the runner
  /// decides it.
  bool moves(Offset point) =>
      band.contains(point) &&
      !holes.any((h) => h.contains(point)) &&
      !buttons.values.any((b) => b.contains(point));

  /// What the app asked of the window, by name.
  List<String> get asked => [
    for (final c in calls)
      if (const {'minimize', 'maximize', 'close'}.contains(c.method)) c.method,
  ];
}

const windows = WindowFrame(
  style: FrameStyle.windows,
  end: WindowButton.values,
);
const mac = WindowFrame(style: FrameStyle.mac);

void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

Future<void> openVault(WidgetTester tester) async {
  openSample();
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text('Backup service'));
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
  await tester.pump();
}

/// Every field and button in the window's top band takes clicks there.
void expectClickable(WidgetTester tester, RunnerStandIn runner, Finder f) {
  for (final element in f.evaluate()) {
    final rect = tester.getRect(find.byElementPredicate((e) => e == element));
    final inBand = rect.intersect(runner.band);
    if (inBand.isEmpty) continue;
    for (final p in [
      inBand.center,
      inBand.topLeft + const Offset(1, 1),
      inBand.bottomRight - const Offset(1, 1),
    ]) {
      expect(
        runner.moves(p),
        isFalse,
        reason: '${element.widget} at $p would move the window',
      );
    }
  }
}

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'Windows: no title bar; minimize, maximize, and close at the top right, '
    'beside the item\'s actions, and the rest of the band moves the window',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      final runner = RunnerStandIn(tester, windows);
      await openVault(tester);

      // The menu, the list's search, and the item's place go up to the
      // window's top, where its title bar was.
      expect(runner.band, const Rect.fromLTWH(0, 0, 1280, windowBand));
      final search = tester.getRect(find.byType(TextField));
      expect(search.top, 12);
      expect(search.center.dy, windowRow);
      expect(tester.getRect(find.byType(Wordmark).first).top, 30);

      // Windows 11's three, 46 by 32, flush with the top right.
      for (final (i, name) in ['minimize', 'maximize', 'close'].indexed) {
        expect(
          runner.buttons[name],
          Rect.fromLTWH(1280 - 46.0 * (3 - i), 0, 46, 32),
          reason: name,
        );
      }
      expect(find.bySemanticsLabel('Minimize'), findsOneWidget);
      expect(find.bySemanticsLabel('Maximize'), findsOneWidget);
      expect(find.bySemanticsLabel('Close'), findsOneWidget);

      // Resting on one (as the runner says, since the mouse there is
      // Windows' to answer for), its name shows under it, and goes as the
      // mouse leaves.
      WindowFrameKeeper.instance.mouse.value = (WindowButton.close, null);
      await tester.pump(const Duration(milliseconds: 200));
      expect(find.text('Close'), findsNothing);
      await tester.pump(const Duration(milliseconds: 400));
      final tip = tester.getRect(find.text('Close'));
      expect(tip.top, greaterThan(32));
      expect(tip.right, lessThanOrEqualTo(1280 - 4));
      expect(tip.right, greaterThan(1280 - 46));
      WindowFrameKeeper.instance.mouse.value = (null, null);
      await tester.pump();
      expect(find.text('Close'), findsNothing);

      await tester.tap(find.text('Backup service').first);
      await tester.pump();
      await tester.pump();
      // The item's actions on the band, in the search's row, short of the
      // window's buttons.
      final edit = tester.getRect(find.widgetWithText(OutlinedButton, 'Edit'));
      expect(edit.center.dy, windowRow);
      final trash = tester.getRect(find.byTooltip('Move to trash'));
      expect(trash.right, lessThanOrEqualTo(1280 - 46 * 3 - 8));
      expect(trash.center.dy, windowRow);

      // Everything that takes clicks on the band still does.
      await tester.pump();
      expectClickable(tester, runner, find.byType(TextField));
      expectClickable(tester, runner, find.byType(IconButton));
      expectClickable(tester, runner, find.byType(OutlinedButton));
      expectClickable(tester, runner, find.byType(FilledButton));
      // And the rest moves the window: over the logo, between the search
      // and the item, and along the top.
      expect(runner.moves(const Offset(100, 40)), isTrue);
      expect(runner.moves(const Offset(6, 6)), isTrue);
      expect(runner.moves(Offset(search.left - 4, 30)), isTrue);
      final favorite = tester.getRect(find.byTooltip('Add to favorites'));
      expect(runner.moves(Offset(favorite.left - 40, 30)), isTrue);
      expect(runner.moves(const Offset(640, 2)), isTrue);
      // But not under the band.
      expect(runner.moves(const Offset(640, windowBand + 1)), isFalse);

      // Maximized, maximize is restore.
      WindowFrameKeeper.instance.debugFrame = const WindowFrame(
        style: FrameStyle.windows,
        maximized: true,
        end: WindowButton.values,
      );
      await tester.pump();
      expect(find.bySemanticsLabel('Restore'), findsOneWidget);
      expect(find.bySemanticsLabel('Maximize'), findsNothing);
    },
  );

  testWidgets('the window\'s buttons are reached with Tab and pressed with '
      'Enter or Space, as a screen reader presses them', (tester) async {
    setSize(tester, const Size(1280, 800));
    final runner = RunnerStandIn(tester, windows);
    openEmpty();
    await tester.pumpWidget(const PassBundleApp());
    await tester.pump();
    Future<String?> tabTo(String label) async {
      for (var i = 0; i < 40; i++) {
        await tester.sendKeyEvent(LogicalKeyboardKey.tab);
        await tester.pump();
        final focused = FocusManager.instance.primaryFocus?.context;
        final named = focused
            ?.findAncestorWidgetOfExactType<Semantics>()
            ?.properties
            .label;
        if (named == label) return label;
      }
      return null;
    }

    expect(await tabTo('Minimize'), 'Minimize');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(runner.asked, ['minimize']);
    expect(await tabTo('Close'), 'Close');
    await tester.sendKeyEvent(LogicalKeyboardKey.space);
    await tester.pump();
    expect(runner.asked, ['minimize', 'close']);

    final semantics = tester.ensureSemantics();
    final node = tester.getSemantics(find.bySemanticsLabel('Maximize'));
    expect(node.flagsCollection.isButton, isTrue);
    expect(node.getSemanticsData().hasAction(SemanticsAction.tap), isTrue);
    tester.semantics.tap(find.semantics.byLabel('Maximize'));
    await tester.pump();
    expect(runner.asked, ['minimize', 'close', 'maximize']);
    semantics.dispose();
  });

  testWidgets(
    'Mac: the traffic lights in the menu\'s top, lined up with the search, '
    'and in full screen, where macOS hides them, no room for them',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      // As a Mac lays the app out: its compact density makes the search
      // field and buttons shorter, and their row stays where the lights are.
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      final runner = RunnerStandIn(tester, mac);
      try {
        await openVault(tester);
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
      // The lights: the close button 26 in (14 into the menu's card), their
      // middle on the search's, in a band 60 tall.
      expect(runner.chrome['lights'], [26.0, windowRow, windowBand]);
      expect(tester.getRect(find.byType(TextField)).center.dy, windowRow);
      expect(
        tester.getRect(find.widgetWithText(FilledButton, 'New')).center.dy,
        windowRow,
      );
      // The menu starts under them.
      expect(
        tester.getRect(find.byType(Wordmark).first).top,
        greaterThanOrEqualTo(windowBand),
      );
      // The app draws no buttons of its own.
      expect(find.bySemanticsLabel('Close'), findsNothing);
      expect(runner.moves(const Offset(150, 20)), isTrue);

      WindowFrameKeeper.instance.debugFrame = const WindowFrame(
        style: FrameStyle.mac,
        fullScreen: true,
      );
      await tester.pump();
      await tester.pump();
      expect(runner.chrome.containsKey('lights'), isFalse);
      expect(runner.band.isEmpty, isTrue);
      expect(tester.getRect(find.byType(Wordmark).first).top, 30);
    },
  );

  testWidgets('signing in and unlocking, and the narrow layout: a strip '
      'along the top, with the lights or the buttons in it', (tester) async {
    setSize(tester, const Size(1280, 800));
    var runner = RunnerStandIn(tester, mac);
    openSample();
    await tester.pumpWidget(const PassBundleApp());
    await tester.pump();
    await tester.pump();
    expect(runner.band, const Rect.fromLTWH(0, 0, 1280, windowStrip));
    expect(runner.chrome['lights'], [12.0, 16.0, windowStrip]);
    // The page sees the strip as padding, as a phone's status bar, and
    // keeps under it.
    expect(
      MediaQuery.paddingOf(tester.element(find.byType(SafeArea).first)).top,
      windowStrip,
    );

    // A window too narrow for the menu, the list, and an item: the vault
    // is a phone's, under the strip, with Windows' buttons in it.
    setSize(tester, const Size(600, 800));
    runner = RunnerStandIn(tester, windows);
    await tester.enterText(find.byType(TextField), samplePassword);
    await tester.tap(find.text('Unlock'));
    await pumpUntil(tester, find.text('Logins'));
    await tester.pump();
    expect(runner.band, const Rect.fromLTWH(0, 0, 600, windowStrip));
    expect(runner.buttons['close'], const Rect.fromLTWH(554, 0, 46, 32));
    for (final text in find.byType(Text).evaluate()) {
      final rect = tester.getRect(find.byElementPredicate((e) => e == text));
      if (rect.isEmpty || rect.width == 0) continue;
      expect(
        rect.top,
        greaterThanOrEqualTo(windowStrip),
        reason: '${text.widget} is under the strip',
      );
    }
  });

  testWidgets('Linux: the buttons the desktop asks for, on its side, and '
      'with them on the left, the menu leaves them its top', (tester) async {
    setSize(tester, const Size(1280, 800));
    final runner = RunnerStandIn(
      tester,
      const WindowFrame(
        style: FrameStyle.linux,
        end: [WindowButton.minimize, WindowButton.maximize, WindowButton.close],
      ),
    );
    await openVault(tester);
    final close = runner.buttons['close']!;
    expect(close.size, const Size(24, 24));
    expect(close.center.dy, windowRow);
    expect(close.right, 1280 - 20);
    expect(
      runner.buttons['minimize']!.right,
      lessThan(runner.buttons['maximize']!.left),
    );
    await tester.tap(find.text('Backup service').first);
    await tester.pump();
    await tester.pump();
    expect(
      tester.getRect(find.byTooltip('Move to trash')).right,
      lessThanOrEqualTo(runner.buttons['minimize']!.left - 8),
    );
    // Clicking one asks the runner.
    await tester.tap(find.bySemanticsLabel('Maximize'));
    await tester.pump();
    expect(runner.asked, ['maximize']);

    // GNOME's own: only close, at the end. Elementary's, and a Mac's:
    // at the start, and the menu starts under them.
    WindowFrameKeeper.instance.debugFrame = const WindowFrame(
      style: FrameStyle.linux,
      start: [WindowButton.close, WindowButton.minimize, WindowButton.maximize],
    );
    await tester.pump();
    await tester.pump();
    expect(
      runner.buttons.keys,
      unorderedEquals(['close', 'minimize', 'maximize']),
    );
    expect(runner.buttons['close']!.left, 26);
    expect(runner.buttons['close']!.center.dy, windowRow);
    expect(
      tester.getRect(find.byType(Wordmark).first).top,
      greaterThanOrEqualTo(windowBand),
    );
  });

  testWidgets('a dialog over the band takes its clicks: the window moves '
      'from it no more', (tester) async {
    setSize(tester, const Size(1280, 800));
    final runner = RunnerStandIn(tester, windows);
    await openVault(tester);
    expect(runner.moves(const Offset(100, 40)), isTrue);
    await tester.tap(find.byTooltip('Settings'));
    for (var i = 0; i < 4; i++) {
      await tester.pump(const Duration(milliseconds: 200));
    }
    expect(runner.moves(const Offset(100, 40)), isFalse);
    expect(runner.moves(const Offset(640, 2)), isFalse);
    // The window's own buttons still answer.
    expect(runner.buttons['close'], const Rect.fromLTWH(1234, 0, 46, 32));
  });

  testWidgets('a phone, and a desktop that keeps its own title bar, are as '
      'they were', (tester) async {
    setSize(tester, const Size(1280, 800));
    final runner = RunnerStandIn(tester, const WindowFrame());
    await openVault(tester);
    expect(runner.calls.where((c) => c.method != 'look'), isEmpty);
    expect(find.bySemanticsLabel('Close'), findsNothing);
    expect(tester.getRect(find.byType(Wordmark).first).top, 30);
    expect(MediaQuery.paddingOf(tester.element(find.byType(TextField))).top, 0);

    // A phone never asks: its runner has no such window.
    debugPhone = true;
    addTearDown(() => debugPhone = null);
    await WindowFrameKeeper.instance.load();
    expect(WindowFrameKeeper.instance.frame.value, const WindowFrame());
    expect(runner.calls.where((c) => c.method == 'frame'), isEmpty);
  });
}
