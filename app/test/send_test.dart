// Send (DESIGN.md, "Send in the apps"): the list, making a text Send and
// copying its link, making a file Send from a file the person picks, with
// how far it is and Cancel, editing a Send (its password kept, changed, or
// taken away), the confirmation before deleting, what the organizations'
// policies leave, and the server's refusals in plain words. Against the
// real core, and for what the server was sent a stand-in for it on this
// computer. Mostly on a phone, whose copying and picking go through its own
// native piece (stood in for here), so nothing lands on this computer's
// clipboard.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/attaching.dart';
import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/vault.dart';
import 'package:passbundle/widgets.dart';

import 'support.dart';

/// A stand-in for PassBundle's server: the sample vault's account
/// (test/sample), with Sends made, edited, and deleted as the server's
/// calls for them do. [bodies] has what each call for a Send sent, and
/// [uploaded] each file Send's file as it arrived (the whole form).
class StandIn {
  StandIn._(this._server);

  final HttpServer _server;
  final asked = <String>[];
  final bodies = <Map<String, dynamic>>[];
  final uploaded = <List<int>>[];

  /// Until it completes, a new file Send isn't answered.
  Completer<void>? holdStart;
  final Map<String, dynamic> sync = jsonDecode(
    File('test/sample/sync.json').readAsStringSync(),
  ) as Map<String, dynamic>;

  /// The server's refusal of a Send made or edited, if any.
  String? refusal;
  var _made = 0;

  String get url => 'http://127.0.0.1:${_server.port}';

  List<Map<String, dynamic>> get sends =>
      ((sync['sends'] as List?) ?? const []).cast<Map<String, dynamic>>();

  /// The person as a member (not an owner) of the sample's organization,
  /// with its Send policies on.
  void policies({bool disableSend = false, bool noHidingEmail = false}) {
    final profile = sync['profile'] as Map<String, dynamic>;
    final org = (profile['organizations'] as List).first as Map;
    org['type'] = 2;
    sync['policies'] = [
      if (disableSend)
        {'organizationId': org['id'], 'type': 6, 'enabled': true},
      if (noHidingEmail)
        {
          'organizationId': org['id'],
          'type': 7,
          'enabled': true,
          'data': {'disableHideEmail': true},
        },
    ];
  }

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

  Map<String, dynamic> _kept(Map<String, dynamic> body, String id) => {
    ...body,
    'id': id,
    'accessId': 'access$id',
    'accessCount': 0,
    'password': body['password'] == null ? null : 'hashed by the server',
    'authType': body['password'] == null ? 2 : 1,
    'revisionDate': '2026-11-09T12:0$_made:00.000Z',
    'object': 'send',
  };

  Future<void> _answer(HttpRequest request) async {
    final path = request.uri.path;
    asked.add('${request.method} $path');
    final List<int> bytes;
    try {
      bytes = await request.fold<List<int>>([], (all, b) => all..addAll(b));
    } on HttpException {
      // An upload stopped partway: the core canceled it.
      return;
    }
    void json(int status, Object? value) {
      request.response.statusCode = status;
      request.response.headers.contentType = ContentType.json;
      if (value != null) request.response.write(jsonEncode(value));
    }

    final one = RegExp(r'^/api/sends/([^/]+)(/remove-password)?$')
        .firstMatch(path);
    final file = RegExp(r'^/api/sends/([^/]+)/file/([^/]+)$').firstMatch(path);
    final body = bytes.isNotEmpty && path.startsWith('/api/sends') && file == null
        ? jsonDecode(utf8.decode(bytes)) as Map<String, dynamic>
        : null;
    if (body != null) bodies.add(body);
    switch (path) {
      case '/api/config':
        json(200, {'version': '2026.6.0'});
      case '/identity/accounts/prelogin':
        json(200, {
          'kdf': 0,
          'kdfIterations': 600000,
          'kdfMemory': null,
          'kdfParallelism': null,
        });
      case '/identity/connect/token':
        json(200, {
          'access_token': 'a.b.c',
          'expires_in': 3600,
          'refresh_token': 'refresh',
          'token_type': 'Bearer',
          'Key': (sync['profile'] as Map)['key'],
        });
      case '/api/sync':
        json(200, sync);
      case '/api/sends' when request.method == 'POST':
        if (refusal != null) {
          json(400, {'message': refusal});
          break;
        }
        final kept = _kept(body!, 'send${_made++}');
        sync['sends'] = [...sends, kept];
        json(200, kept);
      // A file Send: made before its file comes, as the server makes one.
      case '/api/sends/file/v2' when request.method == 'POST':
        await holdStart?.future;
        if (refusal != null) {
          json(400, {'message': refusal});
          break;
        }
        final id = 'send${_made++}';
        final kept = {
          ..._kept(body!, id),
          'file': {
            ...(body['file'] as Map),
            'id': 'file$id',
            'size': '${body['fileLength']}',
            'sizeName': '${body['fileLength']} Bytes',
          },
        }..remove('fileLength');
        sync['sends'] = [...sends, kept];
        json(200, {
          'fileUploadType': 0,
          'object': 'send-fileUpload',
          'url': '/sends/$id/file/file$id',
          'sendResponse': kept,
        });
      case _ when file != null && request.method == 'POST':
        uploaded.add(bytes);
        json(200, null);
      case _ when one != null && request.method == 'PUT':
        final i = sends.indexWhere((s) => s['id'] == one.group(1));
        if (refusal != null || i < 0) {
          json(400, {'message': refusal ?? 'Send not found'});
          break;
        }
        final was = sends[i];
        final kept = one.group(2) != null
            ? {...was, 'password': null, 'authType': 2}
            : {
                ..._kept(body!, was['id'] as String),
                // Only a new password changes it.
                if (body['password'] == null) 'password': was['password'],
                if (body['password'] == null) 'authType': was['authType'],
              };
        sync['sends'] = [...sends]..[i] = kept;
        json(200, kept);
      case _ when one != null && request.method == 'DELETE':
        sync['sends'] = [
          for (final s in sends)
            if (s['id'] != one.group(1)) s,
        ];
        json(200, null);
      default:
        json(404, null);
    }
    await request.response.close();
  }
}

/// Pumps until no slow call is under way (the sync after signing in, say),
/// giving them real time, and lets the screens settle.
Future<void> idle(WidgetTester tester) async {
  for (var i = 0; i < 400 && busy.value; i++) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
    await tester.pump();
  }
  for (var i = 0; i < 4; i++) {
    await tester.pump(const Duration(milliseconds: 250));
  }
}

void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// Pumps until something shows, with the app's clock going on too: an
/// upload's progress is asked on a timer (attaching.dart).
/// Pumps, letting real I/O run, until [done] says so.
Future<void> pumpUntilTrue(
  WidgetTester tester,
  bool Function() done, {
  Duration timeout = const Duration(seconds: 20),
}) async {
  final end = DateTime.now().add(timeout);
  while (DateTime.now().isBefore(end)) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 20)),
    );
    await tester.pump(uploadPoll);
    if (done()) return;
  }
  throw TestFailure('Never happened before the timeout');
}

Future<void> pumpOn(
  WidgetTester tester,
  Finder finder, {
  Duration timeout = const Duration(seconds: 20),
}) async {
  final end = DateTime.now().add(timeout);
  while (DateTime.now().isBefore(end)) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 20)),
    );
    await tester.pump(uploadPoll);
    if (finder.evaluate().isNotEmpty) return;
  }
  throw TestFailure('Never showed up: $finder');
}

/// A file on this computer for the picker to answer.
Map<String, Object?> fileToPick(String name, String contents) {
  final dir = Directory.systemTemp.createTempSync('pb-app-send-');
  addTearDown(() => dir.deleteSync(recursive: true));
  final file = File('${dir.path}/$name')..writeAsStringSync(contents);
  return {'path': file.path, 'name': name};
}

/// The editor's choice of what a new Send shares.
Finder kindChoice(String kind) => find.descendant(
  of: find.byKey(const ValueKey('send-kind')),
  matching: find.text(kind),
);

bool holds(List<int> bytes, List<int> part) {
  for (var i = 0; i + part.length <= bytes.length; i++) {
    var all = true;
    for (var j = 0; j < part.length && all; j++) {
      all = bytes[i + j] == part[j];
    }
    if (all) return true;
  }
  return false;
}

void main() {
  setUpAll(loadFonts);

  /// Signed in to a stand-in for the server, on a phone, on the Send tab.
  Future<(StandIn, PhoneStandIn)> onPhone(
    WidgetTester tester, {
    void Function(StandIn)? first,
  }) async {
    final standIn = await StandIn.start(tester);
    first?.call(standIn);
    final phone = PhoneStandIn(tester);
    openEmpty();
    setSize(tester, const Size(390, 844));
    await tester.pumpWidget(const PassBundleApp());
    await tester.tap(find.text('Self-hosted?'));
    await tester.pump();
    await tester.enterText(
      find.widgetWithText(TextField, 'Server'),
      standIn.url,
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Email'),
      'ana.lopez@example.test',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Master password'),
      samplePassword,
    );
    await tester.tap(find.text('Sign in'));
    await pumpUntil(tester, find.text('Logins'));
    // The offer to unlock with the fingerprint next time, once the sync
    // after signing in is done.
    await idle(tester);
    if (find.text('Not now').evaluate().isNotEmpty) {
      await tester.tap(find.text('Not now'));
      await idle(tester);
    }
    await tester.tap(
      find.descendant(
        of: find.byType(NavigationBar),
        matching: find.text('Send'),
      ),
    );
    await idle(tester);
    return (standIn, phone);
  }

  /// Makes a text Send in the editor, with what's given.
  Future<void> make(
    WidgetTester tester, {
    required String name,
    String text = '4417# then the green button',
    String password = '',
  }) async {
    await tester.tap(find.byKey(const ValueKey('new-send')));
    await idle(tester);
    await tester.enterText(find.byKey(const ValueKey('send-name')), name);
    await tester.enterText(find.byKey(const ValueKey('send-text')), text);
    if (password.isNotEmpty) {
      await tester.enterText(
        find.byKey(const ValueKey('send-password')),
        password,
      );
    }
    await tester.tap(find.text('Save'));
  }

  testWidgets(
    'a text Send is made and its link copied, and the server sees neither its text nor its password',
    (tester) async {
      final (standIn, phone) = await onPhone(tester);
      expect(find.textContaining('No Sends.'), findsOneWidget);
      await make(tester, name: 'Door code', password: 'open sesame');
      await pumpUntil(
        tester,
        find.text(
          'Made the Send and copied its link. It clears in 30 seconds.',
        ),
      );
      await idle(tester);
      // Copied by the phone's own piece, which asks the core for the link.
      expect(phone.copied.single['what'], 'sendLink');
      expect(phone.copied.single['id'], 'send0');
      expect(find.text('Door code'), findsOneWidget);
      expect(find.textContaining('password'), findsWidgets);
      // What went: every field encrypted, the seed wrapped, the password's
      // proof (32 bytes in base64), never the text or the password.
      final sent = standIn.bodies.single;
      final said = jsonEncode(sent);
      expect(said, isNot(contains('open sesame')));
      expect(said, isNot(contains('4417#')));
      expect(said, isNot(contains('Door code')));
      for (final field in [
        sent['name'],
        sent['key'],
        (sent['text'] as Map)['text'],
      ]) {
        expect(field, startsWith('2.'));
      }
      expect(base64.decode(sent['password'] as String), hasLength(32));
      expect(sent['type'], 0);
      expect(sent['emails'], isNull);
    },
  );

  testWidgets(
    'an edit keeps the password unless a new one is typed, and Remove password takes it away',
    (tester) async {
      final (standIn, _) = await onPhone(tester);
      await make(tester, name: 'Door code', password: 'open sesame');
      await pumpUntil(tester, find.text('Door code'));
      await idle(tester);
      final first = standIn.bodies.single['password'];

      await tester.tap(find.text('Door code'));
      await idle(tester);
      expect(find.text('Edit Send'), findsOneWidget);
      expect(find.text('New password (optional)'), findsOneWidget);
      await tester.enterText(
        find.byKey(const ValueKey('send-name')),
        'Gate code',
      );
      await tester.tap(find.text('Save'));
      await pumpUntil(tester, find.text('Saved the changes.'));
      await idle(tester);
      expect(find.text('Gate code'), findsOneWidget);
      // No new password: none was sent, and the server kept its own.
      expect(standIn.bodies.last['password'], isNull);
      expect(standIn.sends.single['authType'], 1);
      // The same seed, wrapped again with a new IV.
      expect(standIn.bodies.last['key'], isNot(standIn.bodies.first['key']));

      await tester.tap(find.text('Gate code'));
      await idle(tester);
      await tester.enterText(
        find.byKey(const ValueKey('send-password')),
        'open sesame 2',
      );
      await tester.tap(find.text('Save'));
      await pumpUntil(tester, find.text('Saved the changes.'));
      await idle(tester);
      expect(standIn.bodies.last['password'], isNot(first));
      expect(
        base64.decode(standIn.bodies.last['password'] as String),
        hasLength(32),
      );

      await tester.tap(find.text('Gate code'));
      await idle(tester);
      await tester.tap(find.text('Remove password'));
      await pumpUntil(
        tester,
        find.text('Removed the password. Anyone with the link can open it.'),
      );
      expect(standIn.asked, contains('PUT /api/sends/send0/remove-password'));
      expect(find.text('Password (optional)'), findsOneWidget);
    },
  );

  testWidgets('deleting a Send asks first, and Cancel keeps it', (
    tester,
  ) async {
    final (standIn, _) = await onPhone(tester);
    await make(tester, name: 'Door code');
    await pumpUntil(tester, find.text('Door code'));
    await idle(tester);
    await tester.tap(find.byTooltip('Delete Door code'));
    await idle(tester);
    expect(find.text('Delete Door code?'), findsOneWidget);
    await tester.tap(find.widgetWithText(TextButton, 'Cancel'));
    await idle(tester);
    expect(standIn.asked.where((a) => a.startsWith('DELETE ')), isEmpty);

    await tester.tap(find.byTooltip('Delete Door code'));
    await idle(tester);
    await tester.tap(find.widgetWithText(FilledButton, 'Delete'));
    await pumpUntil(tester, find.text('Deleted Door code.'));
    expect(standIn.asked, contains('DELETE /api/sends/send0'));
    expect(standIn.sends, isEmpty);
    expect(find.textContaining('No Sends.'), findsOneWidget);
  });

  testWidgets(
    'under "Disable Send" only Delete is offered, and under "Send options" hiding the email isn\'t',
    (tester) async {
      final (standIn, phone) = await onPhone(
        tester,
        first: (s) => s.policies(noHidingEmail: true),
      );
      await tester.tap(find.byKey(const ValueKey('new-send')));
      await idle(tester);
      expect(find.text('Hide my email from whoever opens it'), findsNothing);
      await tester.pageBack();
      await idle(tester);
      await make(tester, name: 'Door code');
      await pumpUntil(tester, find.text('Door code'));
      await idle(tester);
      expect(standIn.bodies.single['hideEmail'], false);

      // Disable Send, from the next sync on.
      standIn.policies(disableSend: true);
      await tester.tap(
        find.descendant(
          of: find.byType(NavigationBar),
          matching: find.text('Items'),
        ),
      );
      await idle(tester);
      expect(await tester.runAsync(Core.sync), isA<Answer>());
      await tester.tap(
        find.descendant(
          of: find.byType(NavigationBar),
          matching: find.text('Send'),
        ),
      );
      await idle(tester);
      expect(
        find.text(
          'An organization you\'re in doesn\'t let you make or edit Sends. You can still delete them.',
        ),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('new-send')), findsNothing);
      // Its row doesn't open the editor; its link and Delete are there.
      await tester.tap(find.text('Door code'));
      await idle(tester);
      expect(find.text('Edit Send'), findsNothing);
      await tester.tap(find.byTooltip('Copy link to Door code'));
      await pumpUntil(
        tester,
        find.text('Copied the link. It clears in 30 seconds.'),
      );
      expect(phone.copied.last['what'], 'sendLink');
      expect(find.byTooltip('Delete Door code'), findsOneWidget);
    },
  );

  /// Opens a new file Send in the editor, and picks what the phone's picker
  /// has next.
  Future<void> newFileSend(WidgetTester tester) async {
    await tester.tap(find.byKey(const ValueKey('new-send')));
    await idle(tester);
    await tester.tap(kindChoice('File'));
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('send-choose-file')));
    await idle(tester);
  }

  testWidgets(
    'a file Send is made from the file the person picks and its link copied, and the server sees neither the file, its name, nor the password',
    (tester) async {
      final (standIn, phone) = await onPhone(tester);
      phone.picks.add(fileToPick('floor plan.pdf', 'the floor plan'));
      await tester.tap(find.byKey(const ValueKey('new-send')));
      await idle(tester);
      await tester.tap(kindChoice('File'));
      await tester.pump();
      // The most it takes, said before a file is chosen.
      expect(
        find.text(
          'Up to 500 MB. It\'s encrypted on this device before it\'s sent.',
        ),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('send-text')), findsNothing);
      await tester.tap(find.byKey(const ValueKey('send-choose-file')));
      await pumpUntil(tester, find.byKey(const ValueKey('send-file')));
      expect(find.text('floor plan.pdf · 14 bytes'), findsOneWidget);
      // Asked of the picker with the most a file Send takes, for a Send.
      expect(phone.pickedWith.single, {'limit': 500 * 1024 * 1024, 'send': true});
      // Named after the file until the person names it.
      expect(find.widgetWithText(TextField, 'floor plan.pdf'), findsOneWidget);
      await tester.enterText(
        find.byKey(const ValueKey('send-name')),
        'Floor plan',
      );
      await tester.enterText(
        find.byKey(const ValueKey('send-password')),
        'open sesame',
      );
      await tester.tap(find.text('Save'));
      await pumpOn(
        tester,
        find.text(
          'Made the Send and copied its link. It clears in 30 seconds.',
        ),
      );
      await idle(tester);
      expect(phone.copied.single['what'], 'sendLink');
      expect(phone.copied.single['id'], 'send0');
      expect(find.text('Floor plan'), findsOneWidget);
      expect(find.textContaining('File · deleted'), findsOneWidget);
      // What went: every field encrypted, the file's name too, the seed
      // wrapped, the password's proof, and the encrypted file's length (49
      // bytes and a block); never the file, its name, or the password.
      final sent = standIn.bodies.single;
      final said = jsonEncode(sent);
      for (final plain in [
        'open sesame',
        'floor plan',
        'Floor plan',
        'the floor plan',
      ]) {
        expect(said, isNot(contains(plain)));
      }
      expect(sent['type'], 1);
      for (final field in [
        sent['name'],
        sent['key'],
        (sent['file'] as Map)['fileName'],
      ]) {
        expect(field, startsWith('2.'));
      }
      expect(sent['text'], isNull);
      expect(sent['fileLength'], 49 + 16);
      expect(base64.decode(sent['password'] as String), hasLength(32));
      final upload = standIn.uploaded.single;
      expect(holds(upload, utf8.encode('the floor plan')), isFalse);
      expect(holds(upload, utf8.encode('floor plan.pdf')), isFalse);
      // Sent under its encrypted name, as the server checks.
      expect(
        holds(upload, utf8.encode((sent['file'] as Map)['fileName'] as String)),
        isTrue,
      );
      // The phone's copy is deleted once the upload ends.
      expect(phone.forgotten, 1);
      expect(uploading.value, isNull);
    },
  );

  testWidgets(
    'a file larger than a file Send takes is refused when it\'s picked, before any of it is read or sent',
    (tester) async {
      final (standIn, phone) = await onPhone(tester);
      // The phone's picker says so before it copies the file.
      phone.picks.add({'tooLarge': true});
      await newFileSend(tester);
      expect(
        find.text('That file is larger than 500 MB, the most PassBundle sends.'),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('send-file')), findsNothing);
      expect(phone.forgotten, 1);
      // A file the picker didn't measure is measured here (sparse: it
      // takes no room on the disk).
      final dir = Directory.systemTemp.createTempSync('pb-app-send-');
      addTearDown(() => dir.deleteSync(recursive: true));
      final large = File('${dir.path}/video.mov');
      large.openSync(mode: FileMode.write)
        ..truncateSync(500 * 1024 * 1024 + 1)
        ..closeSync();
      phone.picks.add({'path': large.path, 'name': 'video.mov'});
      await tester.tap(find.byKey(const ValueKey('send-choose-file')));
      await idle(tester);
      expect(
        find.text('That file is larger than 500 MB, the most PassBundle sends.'),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('send-file')), findsNothing);
      // Nothing goes without a file.
      await tester.enterText(find.byKey(const ValueKey('send-name')), 'Video');
      await tester.tap(find.text('Save'));
      await tester.pump();
      expect(find.text('Choose a file to send.'), findsOneWidget);
      expect(standIn.asked.where((a) => a.contains('/file')), isEmpty);
    },
  );

  testWidgets(
    'Cancel stops a file Send while it goes, and the Send the server made is deleted',
    (tester) async {
      final (standIn, phone) = await onPhone(tester);
      phone.picks.add(fileToPick('codes.txt', 'the codes'));
      await newFileSend(tester);
      standIn.holdStart = Completer<void>();
      await tester.tap(find.text('Save'));
      await pumpOn(tester, find.byKey(const ValueKey('send-upload-progress')));
      expect(find.byKey(const ValueKey('send-choose-file')), findsNothing);
      // The progress shows before the request leaves: wait until the server
      // has it, so it makes a Send that Cancel then has to delete.
      await pumpUntilTrue(
        tester,
        () => standIn.asked.contains('POST /api/sends/file/v2'),
      );
      await tester.tap(find.widgetWithText(TextButton, 'Cancel'));
      await tester.pump();
      expect(find.text('Canceling'), findsOneWidget);
      standIn.holdStart!.complete();
      await pumpOn(tester, find.byKey(const ValueKey('send-choose-file')));
      await idle(tester);
      // The editor stays, saying nothing; the server keeps nothing.
      expect(find.text('New Send'), findsOneWidget);
      expect(find.byKey(const ValueKey('send-upload-progress')), findsNothing);
      expect(standIn.uploaded, isEmpty);
      expect(standIn.asked, contains('DELETE /api/sends/send0'));
      expect(standIn.sends, isEmpty);
      expect(uploading.value, isNull);
      expect(find.byType(SnackBar), findsNothing);
    },
  );

  testWidgets(
    'where the server enforces plans, its own sentence for a file Send on Free is said, and nothing is made',
    (tester) async {
      final (standIn, phone) = await onPhone(tester);
      phone.picks.add(fileToPick('codes.txt', 'the codes'));
      standIn.refusal = 'Sending files is part of Premium. Sending text is free.';
      await newFileSend(tester);
      await tester.tap(find.text('Save'));
      await pumpOn(
        tester,
        find.text(
          'Your server said: Sending files is part of Premium. Sending text is free.',
        ),
      );
      expect(find.text('New Send'), findsOneWidget);
      expect(standIn.sends, isEmpty);
      expect(standIn.uploaded, isEmpty);
    },
  );

  testWidgets('the server\'s refusal is said in plain words', (tester) async {
    final (standIn, _) = await onPhone(tester);
    standIn.refusal = 'Due to an Enterprise Policy, you are only able to delete an existing Send.';
    await make(tester, name: 'Door code');
    await pumpUntil(
      tester,
      find.text(
        'An organization you\'re in doesn\'t let you make or edit Sends. You can still delete them.',
      ),
    );
    // The editor stays open, with what was typed.
    expect(find.text('New Send'), findsOneWidget);
    expect(standIn.sends, isEmpty);
  });

  testWidgets(
    'on a computer, Send opens from the menu in a window of its own',
    (tester) async {
      setSize(tester, const Size(1280, 800));
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      final menu = find.descendant(
        of: find.byType(Sidebar),
        matching: find.text('Send'),
      );
      await tester.scrollUntilVisible(
        menu,
        100,
        scrollable: find
            .descendant(
              of: find.byType(Sidebar),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      await tester.tap(menu);
      await idle(tester);
      expect(find.textContaining('No Sends.'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('new-send')));
      await idle(tester);
      expect(find.text('New Send'), findsWidgets);
      // A file Send is offered, with the most it takes, and nothing goes
      // without a file.
      await tester.tap(kindChoice('File'));
      await tester.pump();
      expect(
        find.text(
          'Up to 500 MB. It\'s encrypted on this device before it\'s sent.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Save'));
      await tester.pump();
      expect(find.text('Choose a file to send.'), findsOneWidget);
      await tester.tap(kindChoice('Text'));
      await tester.pump();
      // Nothing goes without a name and a text.
      await tester.tap(find.text('Save'));
      await pumpUntil(tester, find.text('A Send needs a name.'));
      await tester.enterText(
        find.byKey(const ValueKey('send-name')),
        'Door code',
      );
      await tester.tap(find.text('Save'));
      await pumpUntil(tester, find.text('A text Send needs its text.'));
      await tester.enterText(find.byKey(const ValueKey('send-text')), '4417#');
      // The sample has no server: making one needs it.
      await tester.tap(find.text('Save'));
      await pumpUntil(
        tester,
        find.text('You\'ve been signed out. Sign in again.'),
      );
    },
  );
}
