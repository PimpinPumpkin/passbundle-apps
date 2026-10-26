// Dark mode, graphite glass (theme.dart): its text passes WCAG AA (4.5 to 1)
// on everything it's drawn on, in each dark set: the ground, at the
// brightest point of each of its washes too, and the translucent cards,
// fields, and pills laid over them, as well as the solid dialogs and menus.
// Each page stands on the ground and its washes, and the system's bars
// match the app, light or dark.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/main.dart';
import 'package:passbundle/theme.dart';

import 'support.dart';

double contrast(Color a, Color b) {
  final la = a.computeLuminance(), lb = b.computeLuminance();
  return ((la > lb ? la : lb) + 0.05) / ((la > lb ? lb : la) + 0.05);
}

/// [layers] laid over [under], bottom first.
Color stack(Color under, List<Color> layers) =>
    layers.fold(under, (c, layer) => Color.alphaBlend(layer, c));

String hex(Color c) =>
    '#${(c.toARGB32() & 0xFFFFFF).toRadixString(16).padLeft(6, '0')}';

void main() {
  for (final (name, c) in [
    ('graphite glass', PassBundleColors.graphiteGlass),
    ('plain graphite', PassBundleColors.plainGraphite),
    ('true black', PassBundleColors.trueBlack),
  ]) {
    test('$name: every text on every surface, 4.5 to 1 or more', () {
      // The bare ground, and where each wash is brightest.
      final grounds = {
        'the ground': c.ground,
        for (final (i, w) in c.washes.indexed)
          'wash ${i + 1}': Color.alphaBlend(w.color, c.ground),
      };
      final failures = <String>[];
      void check(String text, Color color, String on, Color surface) {
        final ratio = contrast(color, surface);
        if (ratio < 4.5) {
          failures.add(
            '$text ${hex(color)} on $on ${hex(surface)}: '
            '${ratio.toStringAsFixed(2)}',
          );
        }
      }

      const body = ['text', 'muted', 'accent'];
      Color colorOf(String text) => switch (text) {
        'text' => c.text,
        'muted' => c.muted,
        'accent' => c.accent,
        'heading' => c.heading,
        'error' => c.error!,
        'chip' => c.chipText,
        _ => throw ArgumentError(text),
      };
      for (final MapEntry(key: where, value: g) in grounds.entries) {
        // Over the ground: titles, headings, links, the logo.
        for (final t in [...body, 'heading', 'error']) {
          check(t, colorOf(t), where, g);
        }
        check('the logo', c.wordmarkPass, where, g);
        check('the logo', c.wordmarkBundle, where, g);
        final card = stack(g, [c.pane]);
        final raised = stack(g, [c.raised]);
        final surfaces = {
          'a card': card,
          'the chosen item': raised,
          'the item\'s well': stack(g, [c.raised, c.well]),
          'a field': stack(g, [c.field]),
          'a field on a card': stack(g, [c.pane, c.field]),
          'a button': stack(g, [c.button]),
          'the menu\'s chosen entry': stack(g, [c.pane, c.tint]),
          'a chosen tab or choice': stack(g, [c.pane, c.selected]),
          'a generated password': stack(g, [c.pane, c.sunken]),
          'the banner': card,
        };
        for (final MapEntry(key: on, value: s) in surfaces.entries) {
          for (final t in [...body, 'error']) {
            check(t, colorOf(t), '$on over $where', s);
          }
        }
        check('the logo', c.wordmarkPass, 'the menu over $where', card);
        check('the logo', c.wordmarkBundle, 'the menu over $where', card);
        for (final (on, s) in [
          ('a pill', stack(g, [c.chip])),
          ('a pill on the chosen item', stack(g, [c.raised, c.chip])),
        ]) {
          check('chip', c.chipText, '$on over $where', s);
        }
      }
      // What floats is solid, and the cards in it.
      for (final (on, s) in [
        ('a dialog', c.dialog),
        ('a menu', c.menu),
        ('a dropdown', c.canvas),
        ('a card in a dialog', stack(c.dialog, [c.pane])),
        ('a field in a dialog', stack(c.dialog, [c.field])),
        ('a card in Settings', stack(c.dialogGround, [c.pane])),
      ]) {
        for (final t in [...body, 'error']) {
          check(t, colorOf(t), on, s);
        }
      }
      check('a heading', c.heading, 'Settings', c.dialogGround);
      check('a button\'s text', c.onAccent, 'the accent', c.accent);
      check('a letter', c.letter, 'its tile', c.letterTile);
      check('a snack bar\'s text', c.onInverse!, 'it', c.inverse!);
      check('a snack bar\'s action', brandGreen, 'it', c.inverse!);
      expect(failures, isEmpty, reason: failures.join('\n'));
    });
  }

  setUpAll(loadFonts);

  for (final dark in [true, false]) {
    testWidgets('${dark ? 'dark' : 'light'}: the page on the ground, and the '
        'system\'s bars to match', (tester) async {
      final told = <Object?>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        const MethodChannel('passbundle/window'),
        (call) async {
          told.add(call.arguments);
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          const MethodChannel('passbundle/window'),
          null,
        ),
      );
      openEmpty();
      await tester.pumpWidget(
        PassBundleApp(themeMode: dark ? ThemeMode.dark : ThemeMode.light),
      );
      await tester.pump();
      final c = dark ? PassBundleColors.dark : PassBundleColors.light;
      // In the dark the page is clear, over the ground and its washes; in
      // the light its own solid ground, as before.
      // The ground behind the app, and in the dark the page's own too.
      expect(
        find.ancestor(of: find.byType(Scaffold), matching: find.byType(Ground)),
        dark ? findsNWidgets(2) : findsOneWidget,
      );
      final scaffold = tester.widget<Material>(
        find
            .descendant(
              of: find.byType(Scaffold),
              matching: find.byType(Material),
            )
            .first,
      );
      expect(scaffold.color, dark ? Colors.transparent : c.ground);
      // The phone's bars: see-through, with icons that show on the ground.
      final bars = tester
          .widget<AnnotatedRegion<SystemUiOverlayStyle>>(
            find.byType(AnnotatedRegion<SystemUiOverlayStyle>).first,
          )
          .value;
      expect(bars.statusBarColor, Colors.transparent);
      expect(bars.systemNavigationBarColor, Colors.transparent);
      expect(
        bars.statusBarIconBrightness,
        dark ? Brightness.light : Brightness.dark,
      );
      expect(
        bars.statusBarBrightness,
        dark ? Brightness.dark : Brightness.light,
      );
      expect(
        bars.systemNavigationBarIconBrightness,
        dark ? Brightness.light : Brightness.dark,
      );
      // A computer's title bar: told once, the app's brightness and ground.
      if (Platform.isMacOS || Platform.isWindows) {
        expect(told, [
          {'dark': dark, 'ground': c.ground.toARGB32() & 0xFFFFFF},
        ]);
      }
    });
  }

  test(
    'the dark theme draws from the dark set, and light is the light set',
    () {
      expect(
        passBundleTheme(Brightness.dark).extension<PassBundleColors>(),
        same(PassBundleColors.dark),
      );
      expect(
        passBundleTheme(Brightness.light).extension<PassBundleColors>(),
        same(PassBundleColors.light),
      );
      expect(
        passBundleTheme(Brightness.dark).colorScheme.primary,
        PassBundleColors.dark.accent,
      );
    },
  );
}
