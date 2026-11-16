// A file on its way to an item (docs/core.md, "Attachments") or in a file
// Send (send.dart), for every screen to see: the core reads, encrypts, and
// sends it on a background isolate and says how far it is, and the item's
// pane or the Send's editor shows it, with Cancel. One file at a time, of
// either kind.

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'core.dart';
import 'widgets.dart';

/// The upload under way: which account's item it's for (none for a file
/// Send's file), the file's name, and how far it is (`phase` "reading",
/// "encrypting", "sending", or "finishing", null until the core says; `done`
/// of `total` bytes of that phase, `total` 0 when it has no amount to show).
@immutable
class Uploading {
  const Uploading({
    required this.account,
    required this.item,
    required this.name,
    this.phase,
    this.done = 0,
    this.total = 0,
    this.canceling = false,
  });

  final String account;

  /// The item's id, or empty for a file Send's file.
  final String item;
  final String name;
  final String? phase;
  final int done;
  final int total;

  /// The person pressed Cancel: the core stops at its next piece.
  final bool canceling;

  /// Whether it's for this item of the account the screens show.
  bool isFor(String id) =>
      id.isNotEmpty && item == id && account == Core.account;

  /// Whether it's a file Send's file.
  bool get isSend => item.isEmpty;

  /// How much of its phase is done, from 0 to 1, or null when it has no
  /// amount to show.
  double? get fraction => total > 0 ? (done / total).clamp(0, 1) : null;

  Uploading _with(Map<String, dynamic> progress, {bool? canceling}) =>
      Uploading(
        account: account,
        item: item,
        name: name,
        phase: progress['phase'] as String? ?? phase,
        done: (progress['done'] as num?)?.toInt() ?? done,
        total: (progress['total'] as num?)?.toInt() ?? total,
        canceling: canceling ?? this.canceling,
      );
}

/// The upload under way, or null.
final uploading = ValueNotifier<Uploading?>(null);

/// How often the screens ask the core how far it is.
const uploadPoll = Duration(milliseconds: 150);

/// What the core says when a second file is attached while one uploads.
const anotherUploading =
    'Another file is still uploading. Wait for it to finish, or cancel it.';

/// Attaches the file at [path] to the item [id] of the account the screens
/// show now, as [name]: the core's answer (`_upload`).
Future<Answer> attachFile(Core core, String id, String path, String name) =>
    _upload(
      core,
      Uploading(account: Core.account, item: id, name: name),
      () => Core.attachFile(id, path, name),
    );

/// Makes a file Send of the file at [path], named [name], with the options
/// in [draft] and [password] (empty for none), for the account the screens
/// show now: the core's answer (`_upload`).
Future<Answer> sendFile(
  Core core,
  Map<String, dynamic> draft,
  String path,
  String name,
  String password,
) => _upload(
  core,
  Uploading(account: Core.account, item: '', name: name),
  () => Core.sendFile(draft, path, name, password),
);

/// Runs an upload ([call]): the core's answer. While it runs, [uploading]
/// says how far it is, and the busy flag is up for each moment the upload
/// has the core (`wantsApp`), from before it takes it, so no screen's read
/// waits on it. A copy a phone's picker made is deleted when it ends,
/// however it ends.
Future<Answer> _upload(
  Core core,
  Uploading starting,
  Future<Answer> Function() call,
) async {
  if (uploading.value != null) {
    return Answer({'ok': false, 'error': anotherUploading});
  }
  uploading.value = starting;
  var step = 0;
  void letGo() {
    if (!uploadHasCore) return;
    uploadHasCore = false;
    busy.value = false;
  }

  void poll() {
    final progress = core.uploadProgress();
    final wants = (progress['wantsApp'] as num?)?.toInt() ?? 0;
    if (wants > 0) {
      // Up before the step takes the core; told, it goes on.
      uploadHasCore = true;
      busy.value = true;
      if (wants != step) {
        step = wants;
        core.uploadGo(step);
      }
    } else {
      letGo();
    }
    final now = uploading.value;
    if (now != null && progress['phase'] != null) {
      uploading.value = now._with(progress);
    }
  }

  final timer = Timer.periodic(uploadPoll, (_) => poll());
  try {
    return await call();
  } finally {
    timer.cancel();
    letGo();
    uploading.value = null;
    await Core.forgetPicked();
  }
}

/// The person's Cancel for the upload under way.
void cancelUpload(Core core) {
  final now = uploading.value;
  if (now == null || now.canceling) return;
  core.cancelUpload();
  uploading.value = now._with(const {}, canceling: true);
}
