// Two-step login through the real core (PB_CORE_LIB) to a stand-in for
// PassBundle's server on this computer: the sign-in screen shows the second
// step the server asks for, and each method's answer leaves the core as the
// server reads it (twoFactorProvider and twoFactorToken), or is refused by
// the core before it's sent. The stand-in turns every answer down; the core's
// own tests sign in for real (core/tests/server.rs).
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/main.dart';
import 'package:passbundle/two_step.dart';

import 'support.dart';

const _state = 'Q1w2E3r4T5y6U7i8O9p0AaSsDdFfGgHh';
const _otp = 'cccccbhuinjdtrbftdrldjlrgkdeubffnkhjfjifvlbr';

/// Duo's address as PassBundle's server makes one, coming back to [origin].
String _duoUrl(String origin) {
  String part(Object json) =>
      base64Url.encode(utf8.encode(jsonEncode(json))).replaceAll('=', '');
  final request = [
    part({'alg': 'HS512'}),
    part({
      'response_type': 'code',
      'client_id': 'DIXXXXXXXXXXXXXXXXXX',
      'redirect_uri': '$origin/duo-redirect-connector.html?client=desktop',
      'state': _state,
      'duo_uname': 'ana@example.test',
    }),
    'c2lnbmF0dXJl',
  ].join('.');
  return 'https://api-1a2b3c4d.duosecurity.com/oauth/v1/authorize?response_type=code&client_id=DIXXXXXXXXXXXXXXXXXX&request=$request';
}

/// The stand-in server, in an isolate of its own: prelogin, then the token
/// endpoint asks for the second step (YubiKey, Duo, or an authenticator app)
/// and turns every answer down. Each sign-in's provider and token go on a
/// line of the file it's given ("-" for none).
Future<void> _standIn(List<Object> args) async {
  final (port, log) = (args[0] as SendPort, File(args[1] as String));
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  final origin = 'http://127.0.0.1:${server.port}';
  port.send(server.port);
  await for (final request in server) {
    final body = await utf8.decoder.bind(request).join();
    final response = request.response..headers.contentType = ContentType.json;
    if (request.uri.path == '/identity/accounts/prelogin') {
      response.write(jsonEncode({'kdf': 0, 'kdfIterations': 600000}));
    } else if (request.uri.path == '/identity/connect/token') {
      final form = Uri.splitQueryString(body);
      log.writeAsStringSync(
        '${form['twoFactorProvider'] ?? '-'} ${form['twoFactorToken'] ?? '-'}\n',
        mode: FileMode.append,
        flush: true,
      );
      response.statusCode = 400;
      response.write(
        jsonEncode(
          form.containsKey('twoFactorToken')
              ? {
                  'error': 'invalid_grant',
                  'error_description': 'Two-step token is invalid. Try again.',
                  'ErrorModel': {
                    'Message': 'Two-step token is invalid. Try again.',
                  },
                }
              : {
                  'error': 'invalid_grant',
                  'error_description': 'Two factor required.',
                  'TwoFactorProviders': ['3', '2', '0'],
                  'TwoFactorProviders2': {
                    '0': null,
                    '2': {'AuthUrl': _duoUrl(origin)},
                    '3': {'Nfc': false},
                  },
                },
        ),
      );
    } else {
      response.statusCode = 404;
    }
    await response.close();
  }
}

/// The real core's calls, but Duo's browser comes straight back with [link].
class _BrowserComesBack extends TwoStepCalls {
  const _BrowserComesBack(this.link);

  final String link;

  @override
  Future<String?> duoBrowse(String url) async =>
      url.startsWith('https://api-1a2b3c4d.duosecurity.com/') ? link : null;
}

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'each method\'s answer reaches the server as it reads them, through the real core',
    (tester) async {
      tester.view.physicalSize = const Size(1280, 1000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      openEmpty();
      final log = File(
        '${Directory.systemTemp.createTempSync('pb-two-step-').path}/forms.txt',
      );
      final (standIn, port) = (await tester.runAsync(() async {
        final ports = ReceivePort();
        final standIn = await Isolate.spawn(_standIn, [
          ports.sendPort,
          log.path,
        ]);
        return (standIn, await ports.first as int);
      }))!;
      addTearDown(standIn.kill);
      List<String> sent() =>
          log.existsSync() ? log.readAsLinesSync() : const <String>[];

      await tester.pumpWidget(const PassBundleApp());
      await tester.tap(find.text('Self-hosted?'));
      await tester.pump();
      await tester.enterText(
        find.widgetWithText(TextField, 'Server'),
        'http://127.0.0.1:$port',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Email'),
        'ana@example.test',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        'correct horse battery staple',
      );
      await tester.tap(find.text('Sign in'));
      // Duo is the strongest the stand-in offers.
      await pumpUntil(
        tester,
        find.text('Approve this sign-in with Duo, in your browser.'),
      );
      expect(sent(), ['- -']);

      // A YubiKey's code: one that isn't is refused here, before the server.
      await tester.tap(find.text('Use another way'));
      await tester.pump();
      await tester.tap(find.widgetWithText(OutlinedButton, 'YubiKey'));
      await tester.pump();
      await tester.enterText(
        find.widgetWithText(TextField, 'YubiKey code'),
        '123456',
      );
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.textContaining('That isn\'t a YubiKey\'s code.'));
      expect(sent(), ['- -']);
      await tester.enterText(
        find.widgetWithText(TextField, 'YubiKey code'),
        _otp.toUpperCase(),
      );
      await tester.tap(find.text('Continue'));
      await pumpUntil(
        tester,
        find.textContaining('That YubiKey code didn\'t work.'),
      );
      expect(sent().last, '3 $_otp');

      // A recovery code, as typed off paper.
      await tester.tap(find.text('Use another way'));
      await tester.pump();
      await tester.tap(find.widgetWithText(OutlinedButton, 'Recovery code'));
      await tester.pump();
      await tester.enterText(
        find.widgetWithText(TextField, 'Recovery code'),
        'abcd efgh ijkl',
      );
      await tester.tap(find.text('Continue'));
      await pumpUntil(
        tester,
        find.textContaining('That recovery code isn\'t right.'),
      );
      expect(sent().last, '8 abcdefghijkl');

      // Duo, for the same sign-in, waiting in the core: an answer for another
      // sign-in finishes nothing and isn't sent; this one's goes as the
      // server reads it, "code|state".
      Future<void> duo(String link) async {
        await tester.pumpWidget(
          MaterialApp(
            home: TwoStepScreen(
              offer: const {
                'providers': [3, 2, 0],
                'securityKey': null,
              },
              calls: _BrowserComesBack(link),
              onSignedIn: () {},
              onStartOver: () {},
            ),
          ),
        );
        await tester.tap(find.text('Continue with Duo'));
      }

      await duo('passbundle://duo-callback?code=c0de&state=AnotherSignInsState1');
      await pumpUntil(
        tester,
        find.text('That Duo answer was for another sign-in. Continue with Duo again.'),
      );
      expect(sent().where((l) => l.startsWith('2 ')), isEmpty);
      await duo('passbundle://duo-callback?code=c0de&state=$_state');
      await pumpUntil(
        tester,
        find.text('Duo didn\'t let this sign-in through. Continue with Duo to try again.'),
      );
      expect(sent().last, '2 c0de|$_state');
    },
  );
}
