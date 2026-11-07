// Dark mode, graphite glass (theme.dart): its text passes WCAG AA (4.5 to 1)
// on everything it's drawn on, in each dark set: the ground, at the
// brightest point of each of its washes too, and the translucent cards,
// fields, and pills laid over them, as well as the solid dialogs and menus.
// Each page stands on the ground and its washes, and the system's bars
// match the app, light or dark. Every tile but the logo's is flat, with its
// glyph at 3 to 1 or more on its tint, light and dark.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/glyphs.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/theme.dart';
import 'package:passbundle/widgets.dart';

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

  // Every tile but the logo's is flat (widgets.dart, JewelTile): the jewel's
  // tone, mostly clear, over whatever the tile is on, and the glyph in the
  // tone. The glyph keeps 3 to 1 or more on it, light and dark, on every
  // surface a tile is drawn on.
  final jewels = <String, Jewel>{
    'All items': Jewel.allItems,
    'Favorites': Jewel.favorites,
    'Health': Jewel.health,
    'Expiring soon': Jewel.expiring,
    for (final kind in ItemKind.all) kind.kind: kind,
    ...Categories.all,
  };
  for (final (name, c) in [
    ('light', PassBundleColors.light),
    ('graphite glass', PassBundleColors.graphiteGlass),
    ('plain graphite', PassBundleColors.plainGraphite),
    ('true black', PassBundleColors.trueBlack),
  ]) {
    test('$name: every tile\'s glyph on its tint, 3 to 1 or more', () {
      final grounds = {
        'the ground': c.ground,
        for (final (i, w) in c.washes.indexed)
          'wash ${i + 1}': Color.alphaBlend(w.color, c.ground),
      };
      final surfaces = {
        for (final MapEntry(key: where, value: g) in grounds.entries) ...{
          where: g,
          'a card over $where': stack(g, [c.pane]),
          'the chosen item over $where': stack(g, [c.raised]),
          'the chosen item, picked, over $where': stack(g, [
            c.raised,
            c.selected,
          ]),
          'the menu\'s chosen entry over $where': stack(g, [c.pane, c.tint]),
          'a chosen tab or choice over $where': stack(g, [c.pane, c.selected]),
          'a pill over $where': stack(g, [c.chip]),
          'the item\'s well over $where': stack(g, [c.raised, c.well]),
          'a sunken card over $where': stack(g, [c.pane, c.sunken]),
        },
        'a dialog': c.dialog,
        'a menu': c.menu,
        'a dropdown': c.canvas,
        'a card in a dialog': stack(c.dialog, [c.pane]),
        'a card in Settings': stack(c.dialogGround, [c.pane]),
      };
      final failures = <String>[];
      for (final MapEntry(key: kind, value: jewel) in jewels.entries) {
        final glyph = jewel.color(c.brightness);
        for (final MapEntry(key: on, value: s) in surfaces.entries) {
          final tile = stack(s, [jewel.tint(c.brightness)]);
          final ratio = contrast(glyph, tile);
          if (ratio < 3) {
            failures.add(
              '$kind ${hex(glyph)} on its tile on $on ${hex(tile)}: '
              '${ratio.toStringAsFixed(2)}',
            );
          }
        }
      }
      expect(failures, isEmpty, reason: failures.join('\n'));
    });
  }

  test('a tile\'s tint is its tone at 11% in the light, 14% in the dark', () {
    expect(ItemKind.login.tint(Brightness.light).toARGB32(), 0x1C2448B8);
    expect(ItemKind.login.tint(Brightness.dark).toARGB32(), 0x2493ADFF);
  });

  setUpAll(loadFonts);

  for (final dark in [true, false]) {
    testWidgets('${dark ? 'dark' : 'light'}: an item\'s tile is flat, and only '
        'the logo\'s is glossy', (tester) async {
      final brightness = dark ? Brightness.dark : Brightness.light;
      await tester.pumpWidget(
        MaterialApp(
          theme: passBundleTheme(brightness),
          home: const Scaffold(
            body: Column(
              children: [
                JewelTile(jewel: ItemKind.card, size: 36),
                Wordmark(),
              ],
            ),
          ),
        ),
      );
      final tile = tester.widget<Container>(
        find
            .descendant(
              of: find.byType(JewelTile),
              matching: find.byType(Container),
            )
            .first,
      );
      final decoration = tile.decoration! as BoxDecoration;
      expect(decoration.color, ItemKind.card.tint(brightness));
      expect(decoration.gradient, isNull);
      expect(decoration.boxShadow, isNull);
      // The glyph in the tone, not white, and no sheen or shade over the tint.
      final glyph = tester.widget<Glyph>(
        find.descendant(
          of: find.byType(JewelTile),
          matching: find.byType(Glyph),
        ),
      );
      expect(glyph.color, ItemKind.card.color(brightness));
      expect(
        find.descendant(
          of: find.byType(JewelTile),
          matching: find.byWidgetPredicate(
            (w) =>
                w is DecoratedBox &&
                w.decoration is BoxDecoration &&
                (w.decoration as BoxDecoration).gradient != null,
          ),
        ),
        findsNothing,
      );
      // The logo keeps its sheen and its shadow, emerald in both.
      final logo = tester.widget<Container>(
        find
            .descendant(
              of: find.byType(LogoTile),
              matching: find.byType(Container),
            )
            .first,
      );
      expect((logo.decoration! as BoxDecoration).color, LogoTile.emerald);
      expect((logo.decoration! as BoxDecoration).boxShadow, isNotEmpty);
      expect(
        find.descendant(
          of: find.byType(LogoTile),
          matching: find.byWidgetPredicate(
            (w) =>
                w is DecoratedBox &&
                w.decoration is BoxDecoration &&
                (w.decoration as BoxDecoration).gradient is RadialGradient,
          ),
        ),
        findsOneWidget,
      );
    });
  }

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
