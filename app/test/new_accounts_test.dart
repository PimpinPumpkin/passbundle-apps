// New accounts on a server that isn't taking them (docs/core.md, "Making an
// account"), against the real core and a stand-in for the server on this
// computer. A server that says so with its configuration isn't offered the
// form: the sign-in screen says it where "Create account" would be, in the
// web vault's words, with a link to the website's contact page on
// PassBundle's server and "ask the person who runs it" on anyone else's. A
// server that doesn't say (an older one, or one that can't be reached) is
// offered the form as before. And an email the server turns down at the
// form's first step gets the web vault's sentence under it.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/sign_in.dart';

import 'support.dart';

/// A stand-in for a PassBundle server, as far as making an account goes: its
/// configuration, and its answer to the form's first step. [asked] has each
/// call.
class StandIn {
  StandIn._(this._server);

  final HttpServer _server;
  final asked = <String>[];

  /// What it answers a request for its configuration with. An older server
  /// has no word about new accounts in it.
  Object? config = {'version': '2026.6.0'};

  /// Why it turns down the email at the form's first step: "closed" or
  /// "address", as PassBundle's server says it, or null: an older server,
  /// which gives no reason the apps can read.
  String? refusal;

  String get url => 'http://127.0.0.1:${_server.port}';

  /// A server that says whether it takes new accounts.
  void says({required bool closed}) => config = {
    'version': '2026.6.0',
    'settings': {'disableUserRegistration': closed},
  };

  /// Started, and answering, outside the test's fake clock.
  static Future<StandIn> start(WidgetTester tester) async {
    final standIn = (await tester.runAsync(() async {
      final made = StandIn._(
        await HttpServer.bind(InternetAddress.loopbackIPv4, 0),
      );
      made._server.listen(made._answer);
      return made;
    }))!;
    addTearDown(() => tester.runAsync(standIn.stop));
    return standIn;
  }

  /// Nothing answers there any more: this device can't reach it.
  Future<void> stop() => _server.close(force: true);

  Future<void> _answer(HttpRequest request) async {
    final path = request.uri.path;
    asked.add('${request.method} $path');
    await utf8.decoder.bind(request).join();
    void json(int status, Object? value) {
      request.response.statusCode = status;
      request.response.headers.contentType = ContentType.json;
      if (value != null) request.response.write(jsonEncode(value));
    }

    switch (path) {
      case '/api/config' when config is String:
        // Not a PassBundle server: a web page.
        request.response.headers.contentType = ContentType.html;
        request.response.write(config);
      case '/api/config':
        json(200, config);
      case '/identity/accounts/register/send-verification-email':
        // As PassBundle's server refuses a new account (its
        // new_account_refused), and as an older one does.
        final message = switch (refusal) {
          'closed' => 'This server isn\'t taking new accounts.',
          'address' => 'New accounts aren\'t open for that address.',
          _ => 'Registration not allowed or user already exists',
        };
        json(400, {
          'message': message,
          'validationErrors': {
            '': [message],
          },
          'errorModel': {'message': message, 'object': 'error'},
          'error': '',
          'error_description': '',
          'exceptionMessage': null,
          'exceptionStackTrace': null,
          'innerExceptionMessage': null,
          'keykeeperNewAccounts': ?refusal,
          'object': 'error',
        });
      default:
        json(404, null);
    }
    await request.response.close();
  }
}

// The web vault's sentences (passbundle-clients, brand/messages.en.json, the
// keys starting keykeeperNewAccounts), word for word.
const notOpenYet = 'New accounts aren\'t open yet.';
const askForOne = 'Ask for one';
const ownServer =
    'This server isn\'t taking new accounts. Ask the person who runs it.';
const notThatAddress = 'New accounts aren\'t open for that address.';
const notThatAddressOwnServer =
    'New accounts aren\'t open for that address. Ask the person who runs this server.';

/// The website's contact page, as the core has it from brand.json.
const contactPage = 'https://passbundle.com/contact/';

void main() {
  setUpAll(loadFonts);

  setUp(() {
    // The sign-in screen on a computer, with nothing kept.
    openEmpty();
    addTearDown(() => debugServers = null);
  });

  /// The app, opening on the sign-in screen, with [ours] standing in for
  /// PassBundle's own server, which the screen asks as it opens.
  Future<void> open(
    WidgetTester tester,
    String ours, {
    String? managed,
    String? page = contactPage,
  }) async {
    debugServers = {'ours': ours, 'managed': managed, 'contactPage': page};
    tester.view.physicalSize = const Size(1280, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(const PassBundleApp());
  }

  /// Gives the screen real time for an answer that changes nothing on it:
  /// until the server has been asked, and a little longer.
  Future<void> wait(WidgetTester tester, bool Function() until) async {
    final end = DateTime.now().add(const Duration(seconds: 20));
    var after = 6;
    while (DateTime.now().isBefore(end) && after > 0) {
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 50)),
      );
      await tester.pump();
      if (until()) after--;
    }
    expect(until(), isTrue);
  }

  /// The screen's look at its server, done: asked once more here, and waited
  /// for, where no server is there to say it was asked.
  Future<void> looked(WidgetTester tester) async {
    final screen = tester.state(find.byType(SignInScreen)) as dynamic;
    await tester.runAsync(() async => await screen.askAboutNewAccounts());
    await tester.pump();
  }

  /// The person's own server, typed in, and its field left: that checks it.
  Future<void> typeServer(WidgetTester tester, String server) async {
    if (find.text('Self-hosted?').evaluate().isNotEmpty) {
      await tester.tap(find.text('Self-hosted?'));
      await tester.pump();
    }
    await tester.tap(find.widgetWithText(TextField, 'Server'));
    await tester.enterText(find.widgetWithText(TextField, 'Server'), server);
    await tester.tap(find.widgetWithText(TextField, 'Email'));
    await tester.pump();
  }

  /// The form for making an account, filled in and sent.
  Future<void> makeAccount(WidgetTester tester, String email) async {
    await tester.enterText(find.widgetWithText(TextField, 'Email'), email);
    await tester.enterText(
      find.widgetWithText(TextField, 'Master password'),
      'correct horse battery',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Master password again'),
      'correct horse battery',
    );
    await tester.tap(find.text('Create account'));
    await tester.pump();
  }

  final said = find.byKey(const ValueKey('no-new-accounts'));
  final link = find.byKey(const ValueKey('ask-for-account'));
  Finder inWhatsSaid(String text) =>
      find.descendant(of: said, matching: find.text(text));

  /// Under the form's email field, before the name.
  void expectUnderEmail(WidgetTester tester) {
    final email = tester.getRect(find.widgetWithText(TextField, 'Email'));
    final name = tester.getRect(
      find.widgetWithText(TextField, 'Name (optional)'),
    );
    final words = tester.getRect(said);
    expect(words.top, greaterThanOrEqualTo(email.bottom));
    expect(words.bottom, lessThanOrEqualTo(name.top));
  }

  const configuration = 'GET /api/config';
  const firstStep = 'POST /identity/accounts/register/send-verification-email';

  testWidgets(
    'a server that takes new accounts is offered the form, as before',
    (tester) async {
      final ours = await StandIn.start(tester);
      ours.says(closed: false);
      await open(tester, ours.url);
      await wait(tester, () => ours.asked.isNotEmpty);
      // One look at its configuration, and nothing else.
      expect(ours.asked, [configuration]);
      expect(said, findsNothing);
      expect(find.text(askForOne), findsNothing);

      await tester.tap(find.text('Create account'));
      await tester.pump();
      expect(find.text('Create a PassBundle account'), findsOneWidget);
      expect(said, findsNothing);
    },
  );

  testWidgets(
    'PassBundle\'s server, closed: no form, the web vault\'s sentence, and a link to the website\'s contact page',
    (tester) async {
      final ours = await StandIn.start(tester);
      ours.says(closed: true);
      // The link opens through the phone's own code here, so the test sees
      // the address and no browser opens.
      debugPhone = true;
      addTearDown(() => debugPhone = null);
      final opened = <String>[];
      var opens = true;
      final messenger = tester.binding.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(Core.platform, (call) async {
        if (call.method != 'openUrl') return null;
        opened.add((call.arguments as Map)['url'] as String);
        return opens;
      });
      addTearDown(
        () => messenger.setMockMethodCallHandler(Core.platform, null),
      );

      await open(tester, ours.url);
      await pumpUntil(tester, inWhatsSaid(notOpenYet));
      expect(ours.asked, [configuration]);
      // In place of "Create account"; signing in is as it was.
      expect(find.text('Create account'), findsNothing);
      expect(find.text('Create a PassBundle account'), findsNothing);
      expect(find.text('Sign in to PassBundle'), findsOneWidget);
      expect(find.text('Sign in'), findsOneWidget);
      expect(find.text('Self-hosted?'), findsOneWidget);
      expect(find.text(ownServer), findsNothing);
      final signIn = tester.getRect(find.text('Sign in'));
      final selfHosted = tester.getRect(find.text('Self-hosted?'));
      expect(tester.getRect(said).top, greaterThan(signIn.bottom));
      expect(tester.getRect(said).bottom, lessThan(selfHosted.top));

      // The link, by its words, opens the contact page in the browser.
      expect(
        find.descendant(of: link, matching: find.text(askForOne)),
        findsOneWidget,
      );
      await tester.tap(link);
      await tester.pump();
      expect(opened, [contactPage]);
      expect(find.textContaining('Couldn\'t open'), findsNothing);

      // No browser: the address is said, to type.
      opens = false;
      await tester.tap(link);
      await tester.pump();
      expect(find.text('Couldn\'t open $contactPage.'), findsOneWidget);
      // Nothing more went to the server.
      expect(ours.asked, [configuration]);
    },
  );

  testWidgets('the link to the contact page is https or it isn\'t there', (
    tester,
  ) async {
    final ours = await StandIn.start(tester);
    ours.says(closed: true);
    await open(tester, ours.url, page: 'http://passbundle.com/contact/');
    await pumpUntil(tester, inWhatsSaid(notOpenYet));
    expect(link, findsNothing);
    expect(find.text(askForOne), findsNothing);

    // And with no page at all.
    await tester.pumpWidget(const SizedBox());
    await open(tester, ours.url, page: null);
    await pumpUntil(tester, inWhatsSaid(notOpenYet));
    expect(link, findsNothing);
  });

  testWidgets(
    'someone\'s own server, closed: no form, and the person who runs it is who to ask',
    (tester) async {
      final ours = await StandIn.start(tester);
      ours.says(closed: false);
      final theirs = await StandIn.start(tester);
      theirs.says(closed: true);
      await open(tester, ours.url);
      await wait(tester, () => ours.asked.isNotEmpty);
      expect(find.text('Create account'), findsOneWidget);

      // Leaving the server's field checks it, which is when it says.
      await typeServer(tester, theirs.url);
      await pumpUntil(tester, inWhatsSaid(ownServer));
      expect(theirs.asked, [configuration]);
      expect(find.text('Create account'), findsNothing);
      expect(find.text(notOpenYet), findsNothing);
      // Nobody at PassBundle can open someone else's server: no link.
      expect(link, findsNothing);
      expect(find.text(askForOne), findsNothing);
      // Nothing is wrong with the server's address.
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('server')),
          matching: find.textContaining('PassBundle server'),
        ),
        findsNothing,
      );
      expect(find.text('Sign in'), findsOneWidget);

      // Another address is another server: the words go until it's checked.
      await tester.enterText(
        find.widgetWithText(TextField, 'Server'),
        '${theirs.url}/other',
      );
      await tester.pump();
      expect(said, findsNothing);
      expect(find.text('Create account'), findsOneWidget);
      // The one that said so is remembered.
      await tester.enterText(
        find.widgetWithText(TextField, 'Server'),
        theirs.url,
      );
      await tester.pump();
      expect(inWhatsSaid(ownServer), findsOneWidget);

      // Back on PassBundle's, which takes new accounts.
      await tester.tap(find.text('Use PassBundle\'s server'));
      await tester.pump();
      expect(said, findsNothing);
      expect(find.text('Create account'), findsOneWidget);
    },
  );

  testWidgets(
    'the form goes when its server turns out to take no new accounts',
    (tester) async {
      final ours = await StandIn.start(tester);
      ours.says(closed: false);
      final theirs = await StandIn.start(tester);
      theirs.says(closed: true);
      await open(tester, ours.url);
      await wait(tester, () => ours.asked.isNotEmpty);
      await tester.tap(find.text('Create account'));
      await tester.pump();
      expect(find.text('Create a PassBundle account'), findsOneWidget);

      await typeServer(tester, theirs.url);
      await pumpUntil(tester, inWhatsSaid(ownServer));
      expect(find.text('Create a PassBundle account'), findsNothing);
      expect(find.text('Sign in to PassBundle'), findsOneWidget);
      expect(find.text('Create account'), findsNothing);
      expect(theirs.asked, [configuration]);
    },
  );

  testWidgets(
    'an organization\'s server, closed, is someone\'s own: the person who runs it is who to ask',
    (tester) async {
      final ours = await StandIn.start(tester);
      final managed = await StandIn.start(tester);
      managed.says(closed: true);
      await open(tester, ours.url, managed: managed.url);
      await pumpUntil(tester, inWhatsSaid(ownServer));
      expect(managed.asked, [configuration]);
      expect(ours.asked, isEmpty);
      expect(find.text('Create account'), findsNothing);
      expect(link, findsNothing);
      expect(
        find.text('Your organization\'s server: 127.0.0.1'),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'a server that doesn\'t say is offered the form, and nothing is said about one that can\'t be asked',
    (tester) async {
      // An older server: a configuration with no word about new accounts.
      final ours = await StandIn.start(tester);
      await open(tester, ours.url);
      await wait(tester, () => ours.asked.isNotEmpty);
      expect(ours.asked, [configuration]);
      expect(said, findsNothing);
      expect(find.text('Create account'), findsOneWidget);

      // One whose word isn't a yes or a no.
      ours.config = {
        'version': '2026.6.0',
        'settings': {'disableUserRegistration': 'true'},
      };
      await tester.pumpWidget(const SizedBox());
      await open(tester, ours.url);
      await wait(tester, () => ours.asked.length == 2);
      expect(said, findsNothing);
      expect(find.text('Create account'), findsOneWidget);

      // Something that isn't a PassBundle server.
      ours.config =
          '<!DOCTYPE html><html><body>Welcome to nginx!</body></html>';
      await tester.pumpWidget(const SizedBox());
      await open(tester, ours.url);
      await wait(tester, () => ours.asked.length == 3);
      expect(said, findsNothing);
      expect(find.text('Create account'), findsOneWidget);
      expect(find.textContaining('PassBundle server'), findsNothing);

      // And nothing answering at all: this device is offline, say.
      final gone = ours.url;
      await tester.runAsync(ours.stop);
      await tester.pumpWidget(const SizedBox());
      await open(tester, gone);
      await looked(tester);
      expect(said, findsNothing);
      expect(find.textContaining('couldn\'t connect'), findsNothing);
      expect(find.textContaining('127.0.0.1'), findsNothing);
      await tester.tap(find.text('Create account'));
      await tester.pump();
      expect(find.text('Create a PassBundle account'), findsOneWidget);
    },
  );

  testWidgets(
    'an email PassBundle\'s server turns down gets the web vault\'s sentence under it, with the link, and the form stays',
    (tester) async {
      final ours = await StandIn.start(tester);
      ours.says(closed: false);
      ours.refusal = 'address';
      await open(tester, ours.url);
      await wait(tester, () => ours.asked.isNotEmpty);
      await tester.tap(find.text('Create account'));
      await tester.pump();

      await makeAccount(tester, 'ana@elsewhere.test');
      await pumpUntil(tester, inWhatsSaid(notThatAddress));
      expect(ours.asked.last, firstStep);
      expect(find.text('Create a PassBundle account'), findsOneWidget);
      expectUnderEmail(tester);
      expect(
        find.descendant(of: link, matching: find.text(askForOne)),
        findsOneWidget,
      );
      // Once, in the app's place for it, and nothing in the server's voice.
      expect(find.text(notThatAddress), findsOneWidget);
      expect(find.textContaining('Your server said'), findsNothing);
      // The fields don't keep the master password.
      expect(find.text('correct horse battery'), findsNothing);

      // It was about that email: typing another takes it away.
      await tester.enterText(
        find.widgetWithText(TextField, 'Email'),
        'ana@example.test',
      );
      await tester.pump();
      expect(said, findsNothing);

      // A server that takes only the people it invited says it's closed to
      // everyone else, at the same step.
      ours.refusal = 'closed';
      await makeAccount(tester, 'ana@example.test');
      await pumpUntil(tester, inWhatsSaid(notOpenYet));
      expectUnderEmail(tester);
      expect(link, findsOneWidget);
      expect(find.text('Create a PassBundle account'), findsOneWidget);
      expect(find.textContaining('Your server said'), findsNothing);
    },
  );

  testWidgets(
    'an email someone\'s own server turns down: the person who runs it is who to ask',
    (tester) async {
      final ours = await StandIn.start(tester);
      final theirs = await StandIn.start(tester);
      theirs.says(closed: false);
      theirs.refusal = 'address';
      await open(tester, ours.url);
      await wait(tester, () => ours.asked.isNotEmpty);
      await tester.tap(find.text('Create account'));
      await tester.pump();
      await typeServer(tester, theirs.url);
      await wait(tester, () => theirs.asked.isNotEmpty);

      await makeAccount(tester, 'ana@elsewhere.test');
      await pumpUntil(tester, inWhatsSaid(notThatAddressOwnServer));
      expect(theirs.asked.last, firstStep);
      expectUnderEmail(tester);
      expect(link, findsNothing);
      expect(find.text(askForOne), findsNothing);
      expect(find.text('Create a PassBundle account'), findsOneWidget);
      expect(find.textContaining('Your server said'), findsNothing);

      theirs.refusal = 'closed';
      await makeAccount(tester, 'ana@elsewhere.test');
      await pumpUntil(tester, inWhatsSaid(ownServer));
      expectUnderEmail(tester);
      expect(link, findsNothing);

      // Typing another server takes it away: it was that server's answer.
      await tester.enterText(
        find.widgetWithText(TextField, 'Server'),
        '${theirs.url}/other',
      );
      await tester.pump();
      expect(said, findsNothing);
    },
  );

  testWidgets(
    'an older server\'s refusal, with no reason the app can read, is said as before',
    (tester) async {
      final ours = await StandIn.start(tester);
      await open(tester, ours.url);
      await wait(tester, () => ours.asked.isNotEmpty);
      await tester.tap(find.text('Create account'));
      await tester.pump();
      await makeAccount(tester, 'ana@example.test');
      await pumpUntil(
        tester,
        find.text(
          'Your server said: Registration not allowed or user already exists.',
        ),
      );
      expect(said, findsNothing);
      expect(find.text('Create a PassBundle account'), findsOneWidget);
    },
  );

  test('the contact page is the website\'s, from the brand, over https', () {
    expect(Core().servers()['contactPage'], contactPage);
  });
}
