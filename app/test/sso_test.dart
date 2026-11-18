// Single sign-on from the sign-in screen (sso.dart; DESIGN.md in the meta
// repo, "In the desktop and phone apps"), through the real core
// (PB_CORE_LIB) to a stand-in for PassBundle's server on this computer that
// answers as it does: the email's organization, the SSO identifier checked,
// the code redeemed, and the sample vault's account key, which its master
// password unlocks. The browser is stood in for: it takes the address the
// app opens and sends the server's answer to the core's own listener, as
// the server's page, Microsoft, and the server's callback would. The core's
// own tests sign in against a real server and the Entra stand-in
// (core/tests/server.rs).
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/sso.dart';

import 'support.dart';

const computer = Size(1280, 900);
const phone = Size(390, 844);

const noSuchOrganization =
    'No organization uses that SSO identifier, or its single sign-on is turned off. Check it with your administrator.';
const canceled = 'Single sign-on wasn\'t finished. Try again when you\'re ready.';
const notAMember = 'ana@elsewhere.test isn\'t a member of Acme Dental.';

void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// A stand-in for PassBundle's server: emails at example.test find the
/// organization `acme`, the only SSO identifier it knows; a single sign-on's
/// code signs in to the sample vault's account (test/sample), unless
/// [refuse] says why not; and a password sign-in is answered that an
/// organization requires single sign-on ([linked] false: and the account
/// isn't linked to it yet). [tokens] has each token request's form.
class SsoServer {
  SsoServer._(this._server);

  final HttpServer _server;
  final tokens = <Map<String, String>>[];
  String? refuse;
  bool linked = true;
  final Map<String, dynamic> sync =
      jsonDecode(File('test/sample/sync.json').readAsStringSync())
          as Map<String, dynamic>;

  String get url => 'http://127.0.0.1:${_server.port}';

  static Future<SsoServer> start(WidgetTester tester) async {
    final server = (await tester.runAsync(() async {
      final made = SsoServer._(
        await HttpServer.bind(InternetAddress.loopbackIPv4, 0),
      );
      made._server.listen(made._answer);
      return made;
    }))!;
    addTearDown(() => tester.runAsync(() => server._server.close(force: true)));
    return server;
  }

  Future<void> _answer(HttpRequest request) async {
    final path = request.uri.path;
    final text = await utf8.decoder.bind(request).join();
    var (int status, Object? body) = (404, null);
    if (path == '/api/config') {
      (status, body) = (200, {'version': '2026.6.0'});
    } else if (path == '/api/organizations/domain/sso/verified') {
      final email = (jsonDecode(text) as Map)['email'] as String;
      (status, body) = (
        200,
        {
          'object': 'list',
          'data': [
            if (email.endsWith('@example.test'))
              {'organizationIdentifier': 'acme'},
          ],
        },
      );
    } else if (path == '/identity/sso/prevalidate') {
      (status, body) = request.uri.queryParameters['domainHint'] == 'acme'
          ? (200, {'token': ''})
          : (
              400,
              {
                'message':
                    'No organization uses that SSO identifier, or its single sign-on is turned off.',
              },
            );
    } else if (path == '/identity/accounts/prelogin') {
      (status, body) = (200, {'kdf': 0, 'kdfIterations': 600000});
    } else if (path == '/identity/connect/token') {
      final form = Uri.splitQueryString(text);
      tokens.add(form);
      (status, body) = _token(form);
    } else if (path == '/api/sync') {
      (status, body) = (200, sync);
    }
    request.response.statusCode = status;
    request.response.headers.contentType = ContentType.json;
    if (body != null) request.response.write(jsonEncode(body));
    await request.response.close();
  }

  (int, Object) _token(Map<String, String> form) {
    if (form['grant_type'] == 'password') {
      // As passbundle-server answers a password that checked out, for
      // someone an organization requires single sign-on of.
      const message =
          'Your organization requires you to sign in with single sign-on.';
      return (
        400,
        {
          'error': 'invalid_grant',
          'error_description': message,
          'SsoOrganizationIdentifier': 'acme',
          if (!linked)
            'KeykeeperSsoLinkSession':
                'AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_abcde',
          'ErrorModel': {'Message': message, 'Object': 'error'},
        },
      );
    }
    if (refuse case final message?) {
      return (
        400,
        {
          'error': 'invalid_grant',
          'error_description': message,
          'ErrorModel': {'Message': message, 'Object': 'error'},
        },
      );
    }
    final key = (sync['profile'] as Map)['key'];
    return (
      200,
      {
        'access_token': 'a.b.c',
        'expires_in': 3600,
        'refresh_token': 'refresh',
        'token_type': 'Bearer',
        'Key': key,
        'UserDecryptionOptions': {
          'HasMasterPassword': true,
          'MasterPasswordUnlock': {
            'Kdf': {
              'KdfType': 0,
              'Iterations': 600000,
              'Memory': null,
              'Parallelism': null,
            },
            'MasterKeyWrappedUserKey': key,
            'Salt': 'ana.lopez@example.test',
          },
        },
      },
    );
  }
}

/// The browser: every address the app opens, in turn, and nothing shown.
List<String> standInBrowser() {
  final opened = <String>[];
  Core.debugOpenWebsite = (url) async {
    opened.add(url);
    return true;
  };
  addTearDown(() => Core.debugOpenWebsite = null);
  return opened;
}

/// The browser's part of a sign-in the app opened at [authorize]: the
/// server's page, Microsoft, and the server's callback, which sends the
/// browser to the app's listener with the server's code and the core's
/// state. The listener's answer's status.
Future<int> browserAnswers(String authorize, {String? state}) async {
  final query = Uri.parse(authorize).queryParameters;
  final back = Uri.parse(query['redirect_uri']!).replace(
    queryParameters: {
      'code': 'the-servers-code',
      'state': state ?? query['state']!,
    },
  );
  final client = HttpClient();
  try {
    final response = await (await client.getUrl(back)).close();
    await response.drain<void>();
    return response.statusCode;
  } finally {
    client.close(force: true);
  }
}

String identifierTyped(WidgetTester tester) => tester
    .widget<TextField>(find.byKey(const ValueKey('sso-identifier')))
    .controller!
    .text;

/// The sign-in screen on [server], with [email] typed.
Future<void> signInScreen(
  WidgetTester tester,
  SsoServer server,
  String email,
) async {
  await tester.pumpWidget(const PassBundleApp());
  await tester.ensureVisible(find.text('Self-hosted?'));
  await tester.tap(find.text('Self-hosted?'));
  await tester.pump();
  await tester.enterText(find.widgetWithText(TextField, 'Server'), server.url);
  await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
}

void main() {
  setUpAll(loadFonts);

  testWidgets(
    'Use single sign-on: the email finds the organization, the browser signs in, and the master password unlocks',
    (tester) async {
      setSize(tester, computer);
      openEmpty();
      final server = await SsoServer.start(tester);
      final opened = standInBrowser();
      await signInScreen(tester, server, 'ana.lopez@example.test');
      await tester.tap(find.text('Use single sign-on'));
      await pumpUntil(tester, find.text(ssoInBrowser));

      // The organization the email found, and the address the browser
      // opened: the server's authorize, for this app, back to the core's
      // listener on this computer, with an S256 challenge.
      expect(identifierTyped(tester), 'acme');
      final url = opened.single;
      expect(
        url,
        startsWith('${server.url}/identity/connect/authorize?client_id=desktop&'),
      );
      final query = Uri.parse(url).queryParameters;
      expect(query['domain_hint'], 'acme');
      expect(
        query['redirect_uri'],
        matches(RegExp(r'^http://127\.0\.0\.1:\d+/sso$')),
      );
      expect(query['code_challenge_method'], 'S256');
      expect(query['code_challenge'], hasLength(43));
      expect(query['state'], hasLength(43));

      // Another state at the listener isn't taken, and the sign-in waits on.
      expect(
        await tester.runAsync(() => browserAnswers(url, state: 'guessed')),
        400,
      );
      expect(find.text(ssoInBrowser), findsOneWidget);
      expect(await tester.runAsync(() => browserAnswers(url)), 200);
      await pumpUntil(tester, find.text('Unlock with your master password'));
      expect(find.text('ana.lopez@example.test'), findsWidgets);

      // The code went once, with the verifier, redirect, and client it started with.
      final redeemed = server.tokens.single;
      expect(redeemed['grant_type'], 'authorization_code');
      expect(redeemed['code'], 'the-servers-code');
      expect(redeemed['redirect_uri'], query['redirect_uri']);
      expect(redeemed['client_id'], 'desktop');
      expect(redeemed['code_verifier'], hasLength(43));

      // The master password unlocks; a wrong one says so.
      final field = find.byKey(const ValueKey('sso-master-password'));
      await tester.enterText(field, 'not it');
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('That master password isn\'t right.'));
      await tester.enterText(field, samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Example Mail'));
      expect(server.tokens, hasLength(1));
    },
  );

  testWidgets(
    'an email first, an identifier no organization uses, a canceled browser, and the server\'s refusal are said in words',
    (tester) async {
      setSize(tester, computer);
      openEmpty();
      final server = await SsoServer.start(tester);
      final opened = standInBrowser();
      await signInScreen(tester, server, '');
      await tester.tap(find.text('Use single sign-on'));
      await tester.pump();
      expect(
        find.text('Enter your email first, then choose Use single sign-on.'),
        findsOneWidget,
      );

      // An email no organization verified: the identifier is typed.
      await tester.enterText(
        find.widgetWithText(TextField, 'Email'),
        'ana@elsewhere.test',
      );
      await tester.tap(find.text('Use single sign-on'));
      await pumpUntil(tester, find.byKey(const ValueKey('sso-identifier')));
      await pumpUntil(tester, find.text('Continue'));
      expect(identifierTyped(tester), '');
      expect(opened, isEmpty);
      final identifier = find.byKey(const ValueKey('sso-identifier'));
      await tester.enterText(identifier, 'globex');
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.text(noSuchOrganization));
      expect(opened, isEmpty);

      // The browser, canceled.
      await tester.enterText(identifier, 'Acme');
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.text(ssoInBrowser));
      expect(Uri.parse(opened.single).queryParameters['domain_hint'], 'acme');
      await tester.tap(find.text('Cancel'));
      await pumpUntil(tester, find.text(canceled));
      expect(find.text(ssoInBrowser), findsNothing);

      // Microsoft signed someone in whom the server refuses: its words.
      server.refuse = notAMember;
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.text(ssoInBrowser));
      expect(opened, hasLength(2));
      expect(await tester.runAsync(() => browserAnswers(opened.last)), 200);
      await pumpUntil(tester, find.text('Your server said: $notAMember'));
      expect(find.text('Unlock with your master password'), findsNothing);
      expect(server.tokens, hasLength(1));

      // Back to the master password.
      await tester.tap(find.text('Sign in with your master password'));
      await tester.pump();
      expect(find.widgetWithText(TextField, 'Master password'), findsOneWidget);
    },
  );

  testWidgets(
    'a password sign-in an organization requires single sign-on for goes on to it with its identifier, or says to link in the web vault first',
    (tester) async {
      setSize(tester, computer);
      openEmpty();
      final server = await SsoServer.start(tester);
      final opened = standInBrowser();
      await signInScreen(tester, server, 'ana.lopez@example.test');
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        samplePassword,
      );
      await tester.tap(find.text('Sign in'));
      await pumpUntil(tester, find.text(ssoRequired));
      expect(identifierTyped(tester), 'acme');
      // It waits for the person: nothing opened yet.
      expect(opened, isEmpty);
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.text(ssoInBrowser));
      expect(opened, hasLength(1));
      await tester.tap(find.text('Cancel'));
      await pumpUntil(tester, find.text(canceled));
      await tester.tap(find.text('Back'));
      await tester.pump();

      // Not linked yet: the web vault links the account first.
      server.linked = false;
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        samplePassword,
      );
      await tester.tap(find.text('Sign in'));
      await pumpUntil(
        tester,
        find.textContaining('your account isn\'t linked to it yet'),
      );
      expect(
        find.textContaining('the web vault at ${server.url}'),
        findsOneWidget,
      );
      expect(find.text(ssoRequired), findsNothing);
      expect(opened, hasLength(1));
    },
  );

  testWidgets(
    'on a phone\'s layout, single sign-on signs in and the master password unlocks',
    (tester) async {
      setSize(tester, phone);
      PhoneStandIn(tester, status: 'unavailable');
      openEmpty();
      final server = await SsoServer.start(tester);
      final opened = standInBrowser();
      await signInScreen(tester, server, 'ana.lopez@example.test');
      await tester.ensureVisible(find.text('Use single sign-on'));
      await tester.tap(find.text('Use single sign-on'));
      await pumpUntil(tester, find.text(ssoInBrowser));
      expect(find.text('Cancel'), findsOneWidget);
      expect(await tester.runAsync(() => browserAnswers(opened.single)), 200);
      await pumpUntil(tester, find.text('Unlock with your master password'));
      await tester.enterText(
        find.byKey(const ValueKey('sso-master-password')),
        samplePassword,
      );
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Logins'));
      expect(server.tokens, hasLength(1));
    },
  );
}
