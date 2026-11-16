// Attaching files to items and deleting attachments (docs/core.md,
// "Attachments"): the device's picker (stood in for), the core reading,
// encrypting, and sending the file, how far it is on the item with Cancel,
// what the server refuses said in plain words, and who may do it. Against
// the real core, and for what the server was sent a stand-in for it on
// this computer.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/attaching.dart';
import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/vault.dart';

import 'support.dart';

// The sample vault's items: a login with an attachment, and the
// organization's login.
const mail = '3d4e5f60-7182-4930-abc1-d2e3f4051627';
const officeWifi = '4e5f6071-8293-4a41-bcd2-e3f405162738';
const sharedFolder = '2c3d4e5f-6071-4829-9ab0-c1d2e3f40516';

/// A stand-in for PassBundle's server: the sample vault's account
/// (test/sample), with attachments made, uploaded, and deleted as the
/// server's calls for them do. [asked] has each call; [started] what came
/// with each new attachment, and [uploaded] each file as it arrived.
class StandIn {
  StandIn._(this._server);

  final HttpServer _server;
  final asked = <String>[];
  final started = <Map<String, dynamic>>[];
  final uploaded = <List<int>>[];
  final Map<String, dynamic> sync = jsonDecode(
    File('test/sample/sync.json').readAsStringSync(),
  ) as Map<String, dynamic>;

  /// The server's refusal of a new attachment, if any.
  String? refusal;

  /// Takes the file but never lists it.
  bool forgets = false;

  /// Until it completes, a new attachment isn't answered.
  Completer<void>? holdStart;

  var _made = 0;

  String get url => 'http://127.0.0.1:${_server.port}';

  Map<String, dynamic> item(String id) => (sync['ciphers'] as List)
      .cast<Map<String, dynamic>>()
      .firstWhere((c) => c['id'] == id);

  List<Map<String, dynamic>> attachmentsOf(String id) =>
      ((item(id)['attachments'] as List?) ?? const [])
          .cast<Map<String, dynamic>>();

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

    final start = RegExp(r'^/api/ciphers/([^/]+)/attachment/v2$')
        .firstMatch(path);
    final one = RegExp(r'^/api/ciphers/([^/]+)/attachment/([^/]+)$')
        .firstMatch(path);
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
        json(200, listed);
      case _ when start != null:
        final body = jsonDecode(utf8.decode(bytes)) as Map<String, dynamic>;
        started.add(body);
        await holdStart?.future;
        if (refusal != null) {
          json(400, {'message': refusal});
          break;
        }
        final id = 'made${_made++}abcdef0123456789';
        item(start.group(1)!)['attachments'] = [
          ...attachmentsOf(start.group(1)!),
          {
            'id': id,
            'url': null,
            'fileName': body['fileName'],
            'key': body['key'],
            'size': '${body['fileSize']}',
            'sizeName': '${body['fileSize']} Bytes',
            'object': 'attachment',
            // Not listed until its file arrives.
            'waiting': true,
          },
        ];
        json(200, {
          'object': 'attachment-fileUpload',
          'attachmentId': id,
          'url': '/ciphers/${start.group(1)}/attachment/$id',
          'fileUploadType': 0,
          'cipherResponse': item(start.group(1)!),
        });
      case _ when one != null && request.method == 'POST':
        uploaded.add(bytes);
        for (final a in attachmentsOf(one.group(1)!)) {
          if (a['id'] == one.group(2) && !forgets) a.remove('waiting');
        }
        json(200, null);
      case _ when one != null && request.method == 'DELETE':
        item(one.group(1)!)['attachments'] = [
          for (final a in attachmentsOf(one.group(1)!))
            if (a['id'] != one.group(2)) a,
        ];
        json(200, null);
      default:
        json(404, null);
    }
    await request.response.close();
  }

  /// The sync as the server sends it: an attachment whose file hasn't
  /// arrived isn't listed.
  Map<String, dynamic> get listed => {
    ...sync,
    'ciphers': [
      for (final c in (sync['ciphers'] as List).cast<Map<String, dynamic>>())
        {
          ...c,
          'attachments': [
            for (final a
                in ((c['attachments'] as List?) ?? const [])
                    .cast<Map<String, dynamic>>())
              if (a['waiting'] != true) a,
          ],
        },
    ],
  };
}

void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// The device's picker, stood in for: each pick answers what [picks] has
/// next (null: the person chose none). [forgotten] counts the times the
/// copies a phone's picker made were deleted.
class Picker {
  Picker(WidgetTester tester) {
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(Core.platform, (call) async {
      switch (call.method) {
        case 'pickFile':
          limits.add((call.arguments as Map)['limit'] as int);
          return picks.isEmpty ? null : picks.removeAt(0);
        case 'forgetPicked':
          forgotten++;
          return null;
        default:
          return null;
      }
    });
    addTearDown(() => messenger.setMockMethodCallHandler(Core.platform, null));
  }

  final picks = <Map<String, Object?>?>[];
  final limits = <int>[];
  var forgotten = 0;
}

/// A file on this computer for the picker to answer.
Map<String, Object?> fileToPick(String name, String contents) {
  final dir = Directory.systemTemp.createTempSync('pb-app-attach-');
  addTearDown(() => dir.deleteSync(recursive: true));
  final file = File('${dir.path}/$name')..writeAsStringSync(contents);
  return {'path': file.path, 'name': name};
}

/// Pumps until something shows, with the app's clock going on too: the
/// upload's progress is asked on a timer (attaching.dart).
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

  const computer = Size(1280, 800);

  /// Signed in to a stand-in for the server, on a computer, with the item
  /// [open] open.
  Future<StandIn> signIn(
    WidgetTester tester, {
    String open = 'Example Mail',
  }) async {
    final standIn = await StandIn.start(tester);
    openEmpty();
    setSize(tester, computer);
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
    await pumpUntil(tester, find.text('Example Mail'));
    await pumpUntil(tester, find.byTooltip('Sync'));
    await tester.tap(find.widgetWithText(ItemRow, open));
    await tester.pump();
    return standIn;
  }

  testWidgets(
    'a file the person picks is attached and listed once the server has it, and the server never sees what it says or its name',
    (tester) async {
      final standIn = await signIn(tester);
      final picker = Picker(tester)
        ..picks.add(fileToPick('codes.txt', 'the codes: 0451 8812'));
      await tester.tap(find.byKey(const ValueKey('attach-file')));
      await pumpOn(tester, find.text('Attached codes.txt.'));
      // Asked with the most this device attaches.
      expect(picker.limits, [500 * 1024 * 1024]);
      expect(find.textContaining(RegExp(r'^codes\.txt · ')), findsOneWidget);
      expect(find.text('recovery-codes.txt · 36 bytes'), findsOneWidget);
      // The name and the attachment's key went encrypted; the file, as
      // the encrypted file's layout has it: 49 bytes and a block.
      expect(standIn.started, hasLength(1));
      final start = standIn.started.single;
      expect(start['fileName'], startsWith('2.'));
      expect(start['key'], startsWith('2.'));
      expect(start['fileSize'], 49 + 32);
      final upload = standIn.uploaded.single;
      expect(holds(upload, utf8.encode('the codes')), isFalse);
      expect(holds(upload, utf8.encode('codes.txt')), isFalse);
      // Nothing of it shows as on its way any more, and Attach is back.
      expect(find.byKey(const ValueKey('upload-progress')), findsNothing);
      expect(find.byKey(const ValueKey('attach-file')), findsOneWidget);
      expect(uploading.value, isNull);
    },
  );

  testWidgets(
    'a file the server took but doesn\'t list isn\'t shown as attached, and its record is deleted',
    (tester) async {
      final standIn = await signIn(tester);
      Picker(tester).picks.add(fileToPick('codes.txt', 'the codes'));
      standIn.forgets = true;
      await tester.tap(find.byKey(const ValueKey('attach-file')));
      await pumpOn(tester, find.textContaining('Something PassBundle read'));
      expect(find.textContaining(RegExp(r'^codes\.txt · ')), findsNothing);
      expect(standIn.asked.where((a) => a.startsWith('DELETE ')), hasLength(1));
      expect(standIn.attachmentsOf(mail), hasLength(1));
    },
  );

  testWidgets(
    'what the server refuses is said in plain words, and nothing is attached',
    (tester) async {
      final standIn = await signIn(tester);
      Picker(tester).picks.add(fileToPick('codes.txt', 'the codes'));
      standIn.refusal = 'Attachment storage limit exceeded with this file';
      await tester.tap(find.byKey(const ValueKey('attach-file')));
      await pumpOn(
        tester,
        find.text(
          'Your server doesn\'t have room for this file. Delete some attachments, or ask whoever runs it for more room.',
        ),
      );
      expect(find.textContaining(RegExp(r'^codes\.txt · ')), findsNothing);
      expect(standIn.uploaded, isEmpty);
    },
  );

  testWidgets('Cancel stops an upload, and nothing is attached or said', (
    tester,
  ) async {
    final standIn = await signIn(tester);
    Picker(tester).picks.add(fileToPick('codes.txt', 'the codes'));
    standIn.holdStart = Completer<void>();
    await tester.tap(find.byKey(const ValueKey('attach-file')));
    await pumpOn(tester, find.text('Attaching'));
    expect(find.text('codes.txt'), findsOneWidget);
    expect(find.byKey(const ValueKey('upload-progress')), findsOneWidget);
    // Attach waits while a file is on its way.
    expect(find.byKey(const ValueKey('attach-file')), findsNothing);
    await tester.tap(find.widgetWithText(TextButton, 'Cancel'));
    await tester.pump();
    expect(find.text('Canceling'), findsOneWidget);
    standIn.holdStart!.complete();
    await pumpOn(tester, find.byKey(const ValueKey('attach-file')));
    expect(find.text('Attaching'), findsNothing);
    // The record the server made is deleted, and no file was sent.
    expect(standIn.uploaded, isEmpty);
    expect(standIn.asked.where((a) => a.startsWith('DELETE ')), hasLength(1));
    expect(standIn.attachmentsOf(mail), hasLength(1));
    expect(find.byType(SnackBar), findsNothing);
  });

  testWidgets('deleting an attachment asks first, and Cancel keeps it', (
    tester,
  ) async {
    final standIn = await signIn(tester);
    await tester.tap(find.byTooltip('Delete recovery-codes.txt'));
    await tester.pump();
    expect(find.text('Delete recovery-codes.txt?'), findsOneWidget);
    await tester.tap(find.widgetWithText(TextButton, 'Cancel'));
    await tester.pump();
    expect(standIn.asked.where((a) => a.startsWith('DELETE ')), isEmpty);
    expect(find.text('recovery-codes.txt · 36 bytes'), findsOneWidget);

    await tester.tap(find.byTooltip('Delete recovery-codes.txt'));
    await tester.pump();
    await tester.tap(find.widgetWithText(FilledButton, 'Delete'));
    await pumpUntil(tester, find.text('Deleted recovery-codes.txt.'));
    expect(
      standIn.asked,
      contains('DELETE /api/ciphers/$mail/attachment/a1b2c3d4e5f6a7b8c9d0'),
    );
    expect(find.text('recovery-codes.txt · 36 bytes'), findsNothing);
  });

  testWidgets(
    'a file larger than the app attaches is refused before any of it is read or sent',
    (tester) async {
      final standIn = await signIn(tester);
      final dir = Directory.systemTemp.createTempSync('pb-app-attach-');
      addTearDown(() => dir.deleteSync(recursive: true));
      // Sparse: it takes no room on the disk.
      final large = File('${dir.path}/video.mov');
      final opened = large.openSync(mode: FileMode.write)
        ..truncateSync(500 * 1024 * 1024 + 1);
      opened.closeSync();
      Picker(tester).picks.add({'path': large.path, 'name': 'video.mov'});
      await tester.tap(find.byKey(const ValueKey('attach-file')));
      await pumpOn(
        tester,
        find.text(
          'That file is larger than 500 MB, the most PassBundle attaches.',
        ),
      );
      expect(standIn.started, isEmpty);
    },
  );

  testWidgets('an item the person may only view shows no Attach or Delete', (
    tester,
  ) async {
    setSize(tester, computer);
    final sample =
        jsonDecode(File('test/sample/sync.json').readAsStringSync()) as Map;
    final mailItem = (sample['ciphers'] as List).firstWhere(
      (c) => c['id'] == mail,
    );
    openSample(
      changed: {
        sharedFolder: {'readOnly': true},
        officeWifi: {'attachments': mailItem['attachments']},
      },
    );
    await tester.pumpWidget(const PassBundleApp());
    await tester.enterText(find.byType(TextField), samplePassword);
    await tester.tap(find.text('Unlock'));
    await pumpUntil(
      tester,
      find.textContaining('Sign in again to get changes'),
    );
    await tester.tap(find.widgetWithText(ItemRow, 'Office Wi-Fi'));
    await tester.pump();
    expect(find.text('Attachment'), findsOneWidget);
    expect(find.byKey(const ValueKey('attach-file')), findsNothing);
    expect(
      find.byWidgetPredicate(
        (w) => w is Tooltip && (w.message ?? '').startsWith('Delete '),
      ),
      findsNothing,
    );
    // Their own item takes files.
    await tester.tap(find.widgetWithText(ItemRow, 'Example Mail'));
    await tester.pump();
    expect(find.byKey(const ValueKey('attach-file')), findsOneWidget);
    expect(find.byTooltip('Delete recovery-codes.txt'), findsOneWidget);
  });

  testWidgets(
    'an item that asks for the master password shows Attach once it\'s given',
    (tester) async {
      setSize(tester, computer);
      openSample(asking: {mail});
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(
        tester,
        find.textContaining('Sign in again to get changes'),
      );
      await tester.tap(find.widgetWithText(ItemRow, 'Example Mail'));
      await tester.pump();
      expect(
        find.text('This item asks for your master password.'),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('attach-file')), findsNothing);
      await tester.enterText(
        find.widgetWithText(TextField, 'Master password'),
        samplePassword,
      );
      await tester.tap(find.text('Continue'));
      await pumpUntil(tester, find.byKey(const ValueKey('attach-file')));
      expect(find.byTooltip('Delete recovery-codes.txt'), findsOneWidget);
    },
  );

  testWidgets(
    'on a phone, the copy the picker made is deleted once the upload ends, and one too large is said before it\'s sent',
    (tester) async {
      debugPhone = true;
      addTearDown(() => debugPhone = null);
      setSize(tester, const Size(390, 844));
      openSample();
      final picker = Picker(tester)
        ..picks.add({'tooLarge': true})
        ..picks.add(fileToPick('codes.txt', 'the codes'));
      await tester.pumpWidget(const PassBundleApp());
      await tester.enterText(find.byType(TextField), samplePassword);
      await tester.tap(find.text('Unlock'));
      await pumpUntil(tester, find.text('Example Mail'));
      await tester.tap(find.text('Example Mail'));
      await pumpUntil(tester, find.byKey(const ValueKey('attach-file')));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.byKey(const ValueKey('attach-file')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('attach-file')));
      await pumpOn(
        tester,
        find.text(
          'That file is larger than 500 MB, the most PassBundle attaches.',
        ),
      );
      expect(picker.forgotten, 1);
      // With no server behind the sample, the upload stops at the first
      // call to it; the copy goes all the same.
      await tester.pump(const Duration(seconds: 5));
      await tester.tap(find.byKey(const ValueKey('attach-file')));
      await pumpOn(
        tester,
        find.text('You\'ve been signed out. Sign in again.'),
      );
      expect(picker.forgotten, 2);
      expect(uploading.value, isNull);
    },
  );
}
