// What the launch audit found, fixed, against the real core: the editor asks
// before it throws away changes, and never closes on a click outside it;
// making an account says what's wrong under the field it's about; archived
// items stay out of the lists, search, counts, Health, and reminders; a
// card's editor has only its own expiration date, and keeps any other; the
// editor's secret fields are hidden and kept from the keyboard's learning;
// and slow actions show they're working where the person asked.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';

import 'support.dart';

// The sample vault's items (test/sample).
const exampleMail = '3d4e5f60-7182-4930-abc1-d2e3f4051627';
const backup = '5f607182-93a4-4b52-cde3-f40516273849';
const card = '718293a4-b5c6-4d74-8f01-162738495a6b';

/// When the web vault archived an item, as the server sends it.
const archived = {'archivedDate': '2026-11-01T10:00:00.000Z'};

const computer = Size(1280, 800);
const phone = Size(390, 844);

void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// A stand-in for the server on this computer: the sample vault's account
/// (test/sample), signing anyone in. Each path in [held] waits until the
/// test completes its completer, then answers with what that gives. A saved
/// item replaces the one kept, so the sync after it brings it back. [asked]
/// has each call.
class StandIn {
  StandIn._(this._server);

  final HttpServer _server;
  final asked = <String>[];
  final held = <String, Completer<(int, Object?)>>{};
  final Map<String, dynamic> sync = jsonDecode(
    File('test/sample/sync.json').readAsStringSync(),
  ) as Map<String, dynamic>;

  String get url => 'http://127.0.0.1:${_server.port}';

  static Future<StandIn> start(WidgetTester tester) async {
    final standIn = (await tester.runAsync(() async {
      final made = StandIn._(
        await HttpServer.bind(InternetAddress.loopbackIPv4, 0),
      );
      made._server.listen(made._answer);
      return made;
    }))!;
    addTearDown(
      () => tester.runAsync(() => standIn._server.close(force: true)),
    );
    return standIn;
  }

  /// Holds the next call to a path whose start is [path].
  Completer<(int, Object?)> hold(String path) =>
      held[path] = Completer<(int, Object?)>();

  Future<void> _answer(HttpRequest request) async {
    final path = request.uri.path;
    asked.add('${request.method} $path');
    final text = await utf8.decoder.bind(request).join();
    var (int status, Object? body) = (404, null);
    final waiting = held.entries
        .where((e) => path.startsWith(e.key))
        .firstOrNull;
    if (waiting != null) {
      held.remove(waiting.key);
      (status, body) = await waiting.value.future;
    } else if (path == '/api/config') {
      (status, body) = (200, {'version': '2026.6.0'});
    } else if (path == '/identity/accounts/prelogin') {
      (status, body) = (200, {'kdf': 0, 'kdfIterations': 600000});
    } else if (path == '/identity/connect/token') {
      (status, body) = (
        200,
        {
          'access_token': 'a.b.c',
          'expires_in': 3600,
          'refresh_token': 'refresh',
          'token_type': 'Bearer',
          'Key': (sync['profile'] as Map)['key'],
        },
      );
    } else if (path == '/api/sync') {
      (status, body) = (200, sync);
    } else if (request.method == 'PUT' && path.startsWith('/api/ciphers/')) {
      final id = path.split('/').last;
      final ciphers = (sync['ciphers'] as List).cast<Map<String, dynamic>>();
      final at = ciphers.indexWhere((c) => c['id'] == id);
      final saved = {
        ...ciphers[at],
        ...jsonDecode(text) as Map<String, dynamic>,
        'id': id,
        'revisionDate': '2026-11-13T10:00:00.000Z',
      };
      ciphers[at] = saved;
      (status, body) = (200, saved);
    }
    request.response.statusCode = status;
    request.response.headers.contentType = ContentType.json;
    if (body != null) request.response.write(jsonEncode(body));
    await request.response.close();
  }
}

/// The sample vault, locked, kept on [server] instead of its own.
void openSampleOn(String server) {
  final dir = Directory.systemTemp.createTempSync('pb-app-test-').path;
  final settings = jsonDecode(
    File('test/sample/settings.json').readAsStringSync(),
  ) as Map<String, dynamic>;
  File('$dir/settings.json')
      .writeAsStringSync(jsonEncode({...settings, 'server': server}));
  File('test/sample/sync.json').copySync('$dir/sync.json');
  Core().openAt(dir);
  machineDoesntLock(false);
}

/// The sample vault, unlocked, with no server behind it; [changed] items
/// have those of the server's fields set as given, and [expiring] items
/// expiration dates and reminder days (Core.testExpiration).
Future<void> unlock(
  WidgetTester tester,
  Size size, {
  Map<String, Map<String, dynamic>> changed = const {},
  Map<String, Map<String, String>> expiring = const {},
  String ready = 'Example Mail',
}) async {
  setSize(tester, size);
  openSample(changed: changed);
  Core().testExpiration(expiring);
  await tester.pumpWidget(const PassBundleApp());
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text(ready));
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
}

/// Signed in to [standIn], on a computer, with [expiring] items given
/// expiration dates and reminder days (Core.testExpiration).
Future<void> signIn(
  WidgetTester tester,
  StandIn standIn, {
  Map<String, Map<String, String>> expiring = const {},
}) async {
  openEmpty();
  Core().testExpiration(expiring);
  setSize(tester, computer);
  await tester.pumpWidget(const PassBundleApp());
  await tester.tap(find.text('Self-hosted?'));
  await tester.pump();
  await tester.enterText(find.widgetWithText(TextField, 'Server'), standIn.url);
  await tester.enterText(
    find.widgetWithText(TextField, 'Email'),
    'ana.lopez@example.test',
  );
  await tester.enterText(
    find.widgetWithText(TextField, 'Master password'),
    samplePassword,
  );
  await tester.tap(find.text('Sign in'));
  await pumpUntil(tester, find.text('Example Mail'));
  await pumpUntil(tester, find.byTooltip('Sync'));
}

/// Opens an item's editor from its page.
Future<void> edit(WidgetTester tester, String name) async {
  await tester.tap(find.text(name).last);
  await tester.pumpAndSettle();
  await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
  await tester.pumpAndSettle();
}

Finder get editor => find.byType(Scaffold).last;
Finder get discard => find.text('Discard your changes?');

/// The error shown under a text field, by the field's label.
Finder under(String label, String error) => find.descendant(
  of: find.widgetWithText(TextField, label),
  matching: find.text(error),
);

void main() {
  setUpAll(loadFonts);

  group('the editor', () {
    testWidgets(
      'on a computer, a click outside never closes it, and Escape and Cancel ask first once something changed',
      (tester) async {
        await unlock(tester, computer);
        await edit(tester, 'Example Mail');
        expect(find.text('Edit login'), findsOneWidget);

        // Nothing changed: a click outside does nothing; Escape closes it.
        await tester.tapAt(const Offset(8, 8));
        await tester.pumpAndSettle();
        expect(find.text('Edit login'), findsOneWidget);
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pumpAndSettle();
        expect(find.text('Edit login'), findsNothing);
        expect(discard, findsNothing);

        // Changed: a click outside still does nothing, and asks nothing.
        await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
        await tester.pumpAndSettle();
        await tester.enterText(
          find.widgetWithText(TextField, 'Name'),
          'Example Mail (old)',
        );
        await tester.tapAt(const Offset(8, 8));
        await tester.pumpAndSettle();
        expect(find.text('Edit login'), findsOneWidget);
        expect(discard, findsNothing);

        // Escape asks; "Keep editing" keeps what was typed.
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pumpAndSettle();
        expect(discard, findsOneWidget);
        expect(find.text('Keep editing'), findsOneWidget);
        final red = tester.widget<FilledButton>(
          find.widgetWithText(FilledButton, 'Discard'),
        );
        final context = tester.element(discard);
        expect(
          red.style?.backgroundColor?.resolve({}),
          Theme.of(context).colorScheme.error,
        );
        await tester.tap(find.text('Keep editing'));
        await tester.pumpAndSettle();
        expect(discard, findsNothing);
        expect(
          find.widgetWithText(TextField, 'Example Mail (old)'),
          findsOneWidget,
        );

        // Escape in the question is "Keep editing" too, and a click outside it.
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pumpAndSettle();
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pumpAndSettle();
        expect(discard, findsNothing);
        expect(find.text('Edit login'), findsOneWidget);

        // Cancel asks too; "Discard" closes it, and nothing was saved.
        await tester.tap(find.byTooltip('Cancel'));
        await tester.pumpAndSettle();
        expect(discard, findsOneWidget);
        await tester.tap(find.text('Discard'));
        await tester.pumpAndSettle();
        expect(find.text('Edit login'), findsNothing);
        expect(find.text('Example Mail (old)'), findsNothing);
        expect(Core().item(exampleMail)!['name'], 'Example Mail');

        // A changed setting counts too: the favorite switch.
        await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
        await tester.pumpAndSettle();
        await tester.scrollUntilVisible(
          find.text('Favorite'),
          200,
          scrollable: find
              .descendant(
                of: find.byType(Dialog),
                matching: find.byType(Scrollable),
              )
              .first,
        );
        await tester.tap(find.text('Favorite'));
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Cancel'));
        await tester.pumpAndSettle();
        expect(discard, findsOneWidget);
        await tester.tap(find.text('Discard'));
        await tester.pumpAndSettle();
        expect(find.text('Edit login'), findsNothing);
      },
    );

    testWidgets(
      'a new item\'s name has the keyboard, and Escape closes it while nothing is typed',
      (tester) async {
        await unlock(tester, computer);
        await tester.tap(find.text('New'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Login'));
        await tester.pumpAndSettle();
        expect(find.text('New login'), findsOneWidget);
        final name = tester.widget<EditableText>(
          find.descendant(
            of: find.widgetWithText(TextField, 'Name'),
            matching: find.byType(EditableText),
          ),
        );
        expect(name.focusNode.hasFocus, isTrue);
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pumpAndSettle();
        expect(find.text('New login'), findsNothing);

        // Typed in, Escape asks from the field it was typed in.
        await tester.tap(find.text('New'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Login'));
        await tester.pumpAndSettle();
        await tester.enterText(find.widgetWithText(TextField, 'Name'), 'Bank');
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pumpAndSettle();
        expect(discard, findsOneWidget);
        await tester.tap(find.text('Discard'));
        await tester.pumpAndSettle();
        expect(find.text('New login'), findsNothing);
        expect(Core().list('Bank'), isEmpty);
      },
    );

    testWidgets(
      'on a phone, Back and Cancel close it as it is, and ask first once something changed',
      (tester) async {
        PhoneStandIn(tester, status: 'unavailable');
        await unlock(tester, phone, ready: 'Logins');
        await tester.enterText(find.byType(TextField).first, 'Gate');
        await tester.pumpAndSettle();
        await edit(tester, 'Gate code');
        expect(find.text('Edit note'), findsOneWidget);

        // Unchanged, Back closes it.
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(find.text('Edit note'), findsNothing);
        expect(discard, findsNothing);

        // Changed, Back asks.
        await tester.tap(find.widgetWithText(OutlinedButton, 'Edit'));
        await tester.pumpAndSettle();
        await tester.enterText(
          find.widgetWithText(TextField, 'Notes'),
          '4418#',
        );
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(discard, findsOneWidget);
        await tester.tap(find.text('Keep editing'));
        await tester.pumpAndSettle();
        expect(find.text('Edit note'), findsOneWidget);
        expect(find.widgetWithText(TextField, '4418#'), findsOneWidget);
        await tester.tap(find.byTooltip('Cancel'));
        await tester.pumpAndSettle();
        expect(discard, findsOneWidget);
        await tester.tap(find.text('Discard'));
        await tester.pumpAndSettle();
        expect(find.text('Edit note'), findsNothing);
      },
    );

    testWidgets(
      'every secret field is hidden, and the keyboard neither suggests nor learns it',
      (tester) async {
        await unlock(tester, const Size(1280, 1400));
        // Every field in the editor, by its label, scrolled through from
        // the top: the list builds only what's near the screen.
        Future<Map<String, TextField>> fields() async {
          final list = find
              .descendant(
                of: find.byType(Dialog),
                matching: find.byType(Scrollable),
              )
              .first;
          final found = <String, TextField>{};
          for (var i = 0; i < 12; i++) {
            for (final element in find.byType(TextField).evaluate()) {
              final f = element.widget as TextField;
              final label = find
                  .descendant(of: find.byWidget(f), matching: find.byType(Text))
                  .evaluate()
                  .map((e) => (e.widget as Text).data)
                  .whereType<String>()
                  .firstOrNull;
              found[label ?? '$i'] = f;
            }
            await tester.drag(list, const Offset(0, -200));
            await tester.pumpAndSettle();
          }
          await tester.drag(list, const Offset(0, 5000));
          await tester.pumpAndSettle();
          return found;
        }

        // The secrets named, and no other hidden field, are kept from the
        // keyboard's suggestions, corrections, and learning.
        Future<void> secretsAreKept(List<String> labels) async {
          final all = await fields();
          final hidden = {
            for (final e in all.entries)
              if (e.value.obscureText) e.key,
          };
          expect(hidden, labels.toSet());
          for (final label in labels) {
            final f = all[label]!;
            expect(f.enableSuggestions, isFalse, reason: label);
            expect(f.autocorrect, isFalse, reason: label);
            expect(f.enableIMEPersonalizedLearning, isFalse, reason: label);
          }
          // The rest are as they were.
          for (final e in all.entries) {
            if (labels.contains(e.key)) continue;
            expect(e.value.enableSuggestions, isTrue, reason: e.key);
          }
        }

        // A login: its password, one-time code secret, and hidden field.
        await edit(tester, 'Backup service');
        await secretsAreKept([
          'Password',
          'One-time code secret (optional)',
          'API key',
        ]);
        // Shown, they're still kept from the keyboard; the code's secret too.
        await tester.tap(find.byTooltip('Show').first);
        await tester.pumpAndSettle();
        expect(
          find.widgetWithText(TextField, 'GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ'),
          findsOneWidget,
        );
        final totp = tester.widget<TextField>(
          find.widgetWithText(TextField, 'One-time code secret (optional)'),
        );
        expect(totp.obscureText, isFalse);
        expect(totp.enableSuggestions, isFalse);
        expect(totp.autocorrect, isFalse);
        expect(totp.enableIMEPersonalizedLearning, isFalse);
        await tester.tap(find.byTooltip('Cancel'));
        await tester.pumpAndSettle();

        await edit(tester, 'Everyday card');
        await secretsAreKept(['Number', 'Security code']);
        await tester.tap(find.byTooltip('Cancel'));
        await tester.pumpAndSettle();

        await edit(tester, 'Ana at home');
        await secretsAreKept([
          'Social Security number',
          'Passport number',
          'License number',
        ]);
        await tester.tap(find.byTooltip('Cancel'));
        await tester.pumpAndSettle();
      },
    );
  });

  testWidgets(
    'a card\'s editor has only the card\'s own expiration, and saving keeps any other date it has',
    (tester) async {
      final standIn = await StandIn.start(tester);
      await signIn(
        tester,
        standIn,
        expiring: {
          card: {'expires': '2027-03-31', 'remind': '7,1'},
        },
      );
      setSize(tester, const Size(1280, 1100));
      await edit(tester, 'Everyday card');
      expect(find.text('Edit card'), findsOneWidget);
      expect(find.text('Expiration month'), findsOneWidget);
      expect(find.text('Expiration year'), findsOneWidget);
      expect(find.text('Expires (optional)'), findsNothing);
      expect(find.byKey(const ValueKey('remind-me')), findsNothing);

      // Saved with a new name: what the server has back still has the date
      // the web vault set, and its reminder days, as they were.
      expect(Core().item(card)!['expires'], '2027-03-31');
      Core().testExpiration(const {});
      await tester.enterText(
        find.widgetWithText(TextField, 'Name'),
        'Travel card',
      );
      await tester.tap(find.text('Save'));
      await pumpUntil(tester, find.text('Travel card'));
      await pumpUntil(tester, find.byTooltip('Sync'));
      expect(standIn.asked, contains('PUT /api/ciphers/$card'));
      final saved = Core().item(card)!;
      expect(saved['name'], 'Travel card');
      expect(saved['expires'], '2027-03-31');
      expect((saved['remind'] as Map)['value'], '7,1');

      // Another kind still has its date.
      await edit(tester, 'Gate code');
      expect(find.text('Expires (optional)'), findsOneWidget);
    },
  );

  group('making an account', () {
    Future<void> fill(
      WidgetTester tester, {
      String email = 'ana@example.test',
      String password = '',
      String again = '',
      String hint = '',
    }) async {
      await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        password,
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password again'),
        again,
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Hint (optional)'),
        hint,
      );
      await tester.ensureVisible(
        find.widgetWithText(FilledButton, 'Create account'),
      );
      await tester.tap(find.widgetWithText(FilledButton, 'Create account'));
      await tester.pump();
    }

    testWidgets('says what\'s wrong under the field it\'s about', (
      tester,
    ) async {
      final standIn = await StandIn.start(tester);
      standIn.hold('/identity/accounts/register').complete((
        400,
        {'message': 'Email ana@example.test is already taken.'},
      ));
      setSize(tester, const Size(1280, 1000));
      openEmpty();
      await tester.pumpWidget(const PassBundleApp());
      await tester.tap(find.text('Self-hosted?'));
      await tester.pump();
      await tester.enterText(
        find.widgetWithText(TextField, 'Server'),
        standIn.url,
      );
      await tester.tap(find.text('Create account'));
      await tester.pumpAndSettle();

      await fill(tester, email: '', password: 'short', again: 'short');
      expect(under('Email', 'Enter your email address.'), findsOneWidget);
      expect(
        under('Master password', 'Use at least 12 characters.'),
        findsOneWidget,
      );
      expect(find.text('Use at least 12 characters.'), findsOneWidget);

      await fill(
        tester,
        password: 'correct horse battery',
        again: 'correct horse battery!',
      );
      expect(
        under(
          'Master password again',
          'The two master passwords don\'t match.',
        ),
        findsOneWidget,
      );
      expect(find.text('Enter your email address.'), findsNothing);
      expect(find.text('Use at least 12 characters.'), findsNothing);

      await fill(
        tester,
        password: 'correct horse battery',
        again: 'correct horse battery',
        hint: 'it\'s Correct Horse Battery',
      );
      expect(
        under(
          'Hint (optional)',
          'The hint can\'t give away your master password.',
        ),
        findsOneWidget,
      );
      expect(find.text('The two master passwords don\'t match.'), findsNothing);

      // What the server says about the account: under the email.
      await fill(
        tester,
        password: 'correct horse battery',
        again: 'correct horse battery',
        hint: 'the blue door',
      );
      await pumpUntil(
        tester,
        under(
          'Email',
          'Your server said: Email ana@example.test is already taken.',
        ),
      );
      expect(find.textContaining('already taken'), findsOneWidget);

      // Back to signing in, and to the form again: nothing's said yet.
      await tester.tap(find.text('I have an account: sign in'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Create account'));
      await tester.pumpAndSettle();
      expect(find.textContaining('already taken'), findsNothing);
    });
  });

  testWidgets(
    'archived items aren\'t listed, found, counted, judged by Health, or reminded about',
    (tester) async {
      await unlock(
        tester,
        computer,
        changed: {gateCode: archived, backup: archived},
        expiring: {
          gateCode: {'expires': isoInDays(5)},
          exampleMail: {'expires': isoInDays(5)},
        },
      );
      expect(find.text('Gate code'), findsNothing);
      expect(find.text('Backup service'), findsNothing);
      expect(find.text('Example Mail'), findsWidgets);
      // The sample's seven items, less the two.
      final all = find.ancestor(
        of: find.text('All items'),
        matching: find.byType(Row),
      );
      expect(
        find.descendant(of: all.first, matching: find.text('5')),
        findsOneWidget,
      );
      // Searching for one finds nothing.
      await tester.enterText(find.byType(TextField).first, 'Gate');
      await tester.pumpAndSettle();
      expect(find.text('Gate code'), findsNothing);
      await tester.enterText(find.byType(TextField).first, 'backup');
      await tester.pumpAndSettle();
      expect(find.text('Backup service'), findsNothing);
      await tester.enterText(find.byType(TextField).first, '');
      await tester.pumpAndSettle();

      // Reminders and "Expiring soon": the one in use only.
      await pumpUntil(tester, find.text('1 item expires soon'));
      await tester.tap(find.text('Expiring soon').first);
      await tester.pumpAndSettle();
      expect(find.text('Example Mail'), findsWidgets);
      expect(find.text('Gate code'), findsNothing);
      // Health: nothing of the archived ones.
      await tester.tap(find.text('Health').first);
      await pumpUntil(tester, find.text('Example Mail'));
      expect(find.text('Gate code'), findsNothing);
      expect(find.text('Backup service'), findsNothing);
      // The core lists without them too.
      expect(
        Core().list('').map((i) => i['id']),
        isNot(anyOf(contains(gateCode), contains(backup))),
      );
    },
  );

  testWidgets('saving an attachment spins its button until it\'s done', (
    tester,
  ) async {
    final standIn = await StandIn.start(tester);
    await signIn(tester, standIn);
    final asked = standIn.hold('/api/ciphers/$exampleMail/attachment/');
    await tester.tap(find.text('Example Mail').first);
    await tester.pumpAndSettle();
    final button = find.byTooltip('Save recovery-codes.txt');
    final spinning = find.descendant(
      of: button,
      matching: find.byType(CircularProgressIndicator),
    );
    expect(spinning, findsNothing);
    await tester.tap(button);
    await pumpUntil(tester, spinning);
    expect(find.bySemanticsLabel('Saving recovery-codes.txt'), findsOneWidget);
    // Asked again while it's going, nothing more starts.
    await tester.tap(button);
    await tester.pump();
    asked.complete((404, null));
    await pumpUntilGone(tester, spinning);
    await pumpUntil(tester, find.byType(SnackBar));
    expect(standIn.asked.where((a) => a.contains('/attachment/')).length, 1);
  });

  testWidgets(
    'getting the master password hint spins its link, not Unlock, and a failure says so in its title',
    (tester) async {
      final standIn = await StandIn.start(tester);
      setSize(tester, computer);
      openSampleOn(standIn.url);
      await tester.pumpWidget(const PassBundleApp());
      await pumpUntil(tester, find.text('Unlock PassBundle'));
      final asked = standIn.hold('/api/accounts/password-hint');
      final link = find.byKey(const ValueKey('password-hint'));
      await tester.tap(link);
      final spinning = find.descendant(
        of: link,
        matching: find.byType(CircularProgressIndicator),
      );
      await pumpUntil(tester, spinning);
      // Unlock is as it was: its words, and it works.
      final unlock = find.widgetWithText(FilledButton, 'Unlock');
      expect(unlock, findsOneWidget);
      expect(tester.widget<FilledButton>(unlock).onPressed, isNotNull);
      expect(
        find.descendant(
          of: unlock,
          matching: find.byType(CircularProgressIndicator),
        ),
        findsNothing,
      );
      asked.complete((500, null));
      await pumpUntil(tester, find.text('Couldn\'t get your hint'));
      expect(find.text('Master password hint'), findsNothing);
      expect(
        find.textContaining('Your server had a problem (error 500)'),
        findsOneWidget,
      );
      await tester.tap(find.text('OK'));
      await tester.pumpAndSettle();
      expect(spinning, findsNothing);
      expect(find.text('Get your master password hint'), findsOneWidget);
    },
  );
}
