// Send (DESIGN.md, "Send in the apps"): a text or a file shared through a
// link, encrypted on this device, until a date or a number of times it's
// opened. The list, the editor, Copy link, and Delete, for the account the
// screens show. The core encrypts, checks, and hashes; a file Send's file
// goes up as an attachment does (attaching.dart), with how far it is and
// Cancel in the editor; the link goes to the clipboard from the core as a
// secret, cleared after 30 seconds.

import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'attaching.dart';
import 'core.dart';
import 'glyphs.dart';
import 'theme.dart';
import 'vault.dart' show fileSize, uploadWords;
import 'widgets.dart';

const _months = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
];

/// A server's date, here, for people: "Nov 16, 2026, 3:05 PM".
String sendDate(String? iso) {
  final at = iso == null ? null : DateTime.tryParse(iso)?.toLocal();
  if (at == null) return '';
  final hour = at.hour % 12 == 0 ? 12 : at.hour % 12;
  final minute = at.minute.toString().padLeft(2, '0');
  return '${_months[at.month - 1]} ${at.day}, ${at.year}, $hour:$minute ${at.hour < 12 ? 'AM' : 'PM'}';
}

/// A note at the bottom, wherever the person is, in place of the one
/// showing: the newest is the one that matters.
void _say(ScaffoldMessengerState messenger, String text) => messenger
  ..clearSnackBars()
  ..removeCurrentSnackBar()
  ..showSnackBar(SnackBar(content: Text(text)));

/// Copies a Send's link through the core, as a secret.
Future<void> copySendLink(
  Core core,
  ScaffoldMessengerState messenger,
  String id, {
  String? said,
}) async {
  final answer = await core.copy(id, 'sendLink');
  _say(
    messenger,
    answer.ok
        ? '${said ?? 'Copied the link.'} It clears in 30 seconds.'
        : answer.error ?? 'Couldn\'t copy the link.',
  );
}

/// The account's Sends: a phone's Send tab, or the computer's Send window.
class SendPage extends StatefulWidget {
  const SendPage({super.key, required this.core, this.title = true});

  final Core core;

  /// A page's big title (a phone's tab); a dialog has its own.
  final bool title;

  @override
  State<SendPage> createState() => _SendPageState();
}

class _SendPageState extends State<SendPage> {
  Map<String, dynamic>? data;

  @override
  void initState() {
    super.initState();
    busy.addListener(settled);
    reload();
  }

  @override
  void dispose() {
    busy.removeListener(settled);
    super.dispose();
  }

  /// Once a slow call (a sync, say) is done, the Sends as they are now.
  void settled() {
    if (!busy.value && mounted) reload();
  }

  void reload() {
    if (busy.value) return;
    setState(() => data = widget.core.sends());
  }

  Map<String, dynamic> get policy =>
      (data?['policy'] as Map?)?.cast<String, dynamic>() ?? const {};

  bool get disabled => policy['disabled'] == true;

  Future<void> edit([Map<String, dynamic>? send]) async {
    final messenger = ScaffoldMessenger.of(context);
    final saved = await openSendEditor(
      context,
      core: widget.core,
      id: send?['id'] as String?,
      noHidingEmail: policy['noHidingEmail'] == true,
    );
    if (!mounted) return;
    reload();
    if (saved == null) return;
    final unsynced = saved.synced
        ? ''
        : ' PassBundle couldn\'t sync the rest just now, and will next time.';
    if (send == null) {
      // A new one's link goes to the clipboard at once, as upstream's apps
      // do: it's what the person makes it for.
      await copySendLink(
        widget.core,
        messenger,
        saved.id,
        said: 'Made the Send and copied its link.$unsynced',
      );
    } else {
      _say(messenger, 'Saved the changes.$unsynced');
    }
  }

  Future<void> delete(Map<String, dynamic> send) async {
    final name = send['name'] as String;
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Delete $name?'),
        content: const Text(
          'Its link stops working, for everyone. It can\'t be restored.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Delete'),
          ),
        ],
      ),
    );
    if (sure != true || !mounted) return;
    final messenger = ScaffoldMessenger.of(context);
    final answer = await whileBusy(() => Core.deleteSend(send['id'] as String));
    if (!mounted) return;
    reload();
    _say(
      messenger,
      answer.ok
          ? (answer.body['synced'] == false
                ? 'Deleted $name. PassBundle couldn\'t sync the rest just now, and will next time.'
                : 'Deleted $name.')
          : answer.error ?? 'Couldn\'t delete it.',
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final sends = ((data?['sends'] as List?) ?? const [])
        .cast<Map<String, dynamic>>();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (widget.title) const PageTitle('Send'),
        Expanded(
          child: ListView(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 32),
            children: [
              if (disabled)
                Padding(
                  padding: const EdgeInsets.fromLTRB(4, 4, 4, 12),
                  child: Text(
                    'An organization you\'re in doesn\'t let you make or edit Sends. You can still delete them.',
                    style: TextStyle(color: c.muted),
                  ),
                )
              else
                Align(
                  alignment: Alignment.centerLeft,
                  child: Padding(
                    padding: const EdgeInsets.only(bottom: 12),
                    child: FilledButton.icon(
                      key: const ValueKey('new-send'),
                      onPressed: data == null ? null : () => edit(),
                      icon: Glyph(
                        Glyphs.plus,
                        size: 18,
                        strokeWidth: 2.2,
                        color: Theme.of(context).colorScheme.onPrimary,
                      ),
                      label: const Text('New Send'),
                    ),
                  ),
                ),
              if (sends.isEmpty)
                Padding(
                  padding: const EdgeInsets.fromLTRB(4, 8, 4, 0),
                  child: Text(
                    'No Sends. A Send shares a text or a file through a link, encrypted on this device. It\'s deleted on the date you choose, and can have a password.',
                    style: TextStyle(color: c.muted),
                  ),
                )
              else
                RestingCard(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      for (var i = 0; i < sends.length; i++) ...[
                        if (i > 0) const Divider(indent: 16),
                        _SendRow(
                          send: sends[i],
                          editable: !disabled,
                          onEdit: () => edit(sends[i]),
                          onCopy: () => copySendLink(
                            widget.core,
                            ScaffoldMessenger.of(context),
                            sends[i]['id'] as String,
                          ),
                          onDelete: () => delete(sends[i]),
                        ),
                      ],
                    ],
                  ),
                ),
            ],
          ),
        ),
      ],
    );
  }
}

/// One Send in the list: what it is, when it's deleted, what's set, and
/// Copy link and Delete. A tap edits it, where it may be edited.
class _SendRow extends StatelessWidget {
  const _SendRow({
    required this.send,
    required this.editable,
    required this.onEdit,
    required this.onCopy,
    required this.onDelete,
  });

  final Map<String, dynamic> send;
  final bool editable;
  final VoidCallback onEdit;
  final VoidCallback onCopy;
  final VoidCallback onDelete;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final scheme = Theme.of(context).colorScheme;
    final opens = send['opens'] == true;
    final most = send['maxAccessCount'] as num?;
    final about = [
      send['kind'] == 'file' ? 'File' : 'Text',
      if (send['deletionDate'] case final String d) 'deleted ${sendDate(d)}',
      if (send['hasPassword'] == true) 'password',
      if (send['disabled'] == true) 'off',
      if (send['expired'] == true) 'expired',
      if (most != null) 'opened ${send['accessCount']} of $most times',
    ].join(' · ');
    final mayEdit = editable && opens && send['asksForEmail'] != true;
    return InkWell(
      onTap: mayEdit ? onEdit : null,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 10, 8, 10),
        child: Row(
          children: [
            Glyph(
              send['kind'] == 'file' ? Glyphs.note : Glyphs.send,
              size: 22,
              color: scheme.primary,
            ),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    send['name'] as String,
                    style: const TextStyle(
                      fontSize: 16,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: 2),
                  Text(about, style: TextStyle(color: c.muted, fontSize: 13)),
                ],
              ),
            ),
            if (opens)
              IconButton(
                tooltip: 'Copy link to ${send['name']}',
                icon: Glyph(Glyphs.copy, size: 20, color: scheme.primary),
                onPressed: onCopy,
              ),
            IconButton(
              tooltip: 'Delete ${send['name']}',
              icon: Icon(Icons.delete_outline_rounded, color: c.muted),
              onPressed: onDelete,
            ),
          ],
        ),
      ),
    );
  }
}

/// A Send the server took, from the editor.
typedef SavedSend = ({String id, bool synced});

/// The editor for a new Send, of a text or a file ([id] null), or an
/// existing Send, as a dialog on a computer and a page of its own on a
/// phone: what it saved, or null.
Future<SavedSend?> openSendEditor(
  BuildContext context, {
  required Core core,
  String? id,
  bool noHidingEmail = false,
}) {
  final editor = SendEditor(core: core, id: id, noHidingEmail: noHidingEmail);
  if (isPhone) {
    return Navigator.of(context)
        .push(MaterialPageRoute<SavedSend>(builder: (_) => editor));
  }
  return showDialog<SavedSend>(
    context: context,
    builder: (context) => Dialog(
      clipBehavior: Clip.antiAlias,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 520, maxHeight: 760),
        child: editor,
      ),
    ),
  );
}

/// The date a Send has now, kept as it is (an edit).
const _keep = Duration(microseconds: -1);

/// No expiration date: it opens until it's deleted.
const _never = Duration.zero;

/// How long until a Send is deleted, or expires: the presets the web
/// vault offers.
const _presets = <(String, Duration)>[
  ('1 hour', Duration(hours: 1)),
  ('1 day', Duration(days: 1)),
  ('2 days', Duration(days: 2)),
  ('3 days', Duration(days: 3)),
  ('7 days', Duration(days: 7)),
  ('14 days', Duration(days: 14)),
  ('30 days', Duration(days: 30)),
];

class SendEditor extends StatefulWidget {
  const SendEditor({
    super.key,
    required this.core,
    this.id,
    this.noHidingEmail = false,
  });

  final Core core;
  final String? id;

  /// An organization's "Send options" don't let Sends hide the email.
  final bool noHidingEmail;

  @override
  State<SendEditor> createState() => _SendEditorState();
}

class _SendEditorState extends State<SendEditor> {
  final name = TextEditingController();
  final text = TextEditingController();
  final notes = TextEditingController();
  final most = TextEditingController();
  final password = TextEditingController();
  Map<String, dynamic>? send;
  bool hidden = false;
  bool hideEmail = false;
  bool off = false;

  /// The deletion and expiration presets chosen, or the Send's own date
  /// kept (`_keep`), or no expiration (`_never`).
  Duration deleteIn = const Duration(days: 7);
  Duration expireIn = _never;
  String? error;
  bool saving = false;

  /// What it shares: "text" or "file". A Send's kind can't change, so only
  /// a new one offers the choice.
  String kind = 'text';

  /// The file chosen for a new file Send: `{path, name, size}` (`size` null
  /// when this device can't say).
  Map<String, dynamic>? picked;

  /// The largest file a file Send takes from this device, `{bytes,
  /// refusal}`, said before a file is chosen.
  late final Map<String, dynamic> limit = widget.core.sendFileLimit();

  bool get editing => widget.id != null;

  bool get newFile => !editing && kind == 'file';

  @override
  void initState() {
    super.initState();
    if (widget.id case final id?) {
      final s = widget.core.send(id);
      send = s;
      if (s != null) {
        kind = s['kind'] as String? ?? 'text';
        name.text = s['name'] as String? ?? '';
        text.text = s['text'] as String? ?? '';
        notes.text = s['notes'] as String? ?? '';
        most.text = (s['maxAccessCount'] as num?)?.toString() ?? '';
        hidden = s['hidden'] == true;
        hideEmail = s['hideEmail'] == true;
        off = s['disabled'] == true;
        deleteIn = _keep;
        if (s['expirationDate'] != null) expireIn = _keep;
      }
    }
  }

  @override
  void dispose() {
    for (final c in [name, text, notes, most, password]) {
      c.dispose();
    }
    // A phone's copy of a file chosen and not sent, unless another file is
    // uploading from that folder now.
    if (picked != null && uploading.value == null) {
      unawaited(Core.forgetPicked());
    }
    super.dispose();
  }

  String _after(Duration d) => DateTime.now().toUtc().add(d).toIso8601String();

  /// The limit in words: "500 MB".
  String get limitWords =>
      '${((limit['bytes'] as num? ?? 0) / (1024 * 1024)).round()} MB';

  /// Asks for the file with the device's picker. A file larger than a file
  /// Send takes is said at once, before anything is read or sent.
  Future<void> chooseFile() async {
    if (uploading.value != null) {
      setState(() => error = anotherUploading);
      return;
    }
    // A phone keeps one copy at a time: the last one chosen goes first.
    if (picked != null && isPhone) {
      await Core.forgetPicked();
      if (!mounted) return;
      setState(() => picked = null);
    }
    final largest = (limit['bytes'] as num? ?? 0).toInt();
    Map<String, dynamic>? chosen;
    try {
      chosen = await Core.pickFile(largest, send: true);
    } on PlatformException catch (e) {
      chosen = {'error': e.message};
    } on MissingPluginException {
      chosen = {'error': null};
    }
    if (chosen == null) return;
    final path = chosen['path'] as String?;
    final fileName = chosen['name'] as String? ?? '';
    final why = chosen['error'] as String?;
    int? size;
    if (path != null) {
      try {
        size = File(path).lengthSync();
      } on FileSystemException {
        size = null;
      }
    }
    final tooLarge = chosen['tooLarge'] == true || (size ?? 0) > largest;
    if (!mounted || path == null || tooLarge) {
      await Core.forgetPicked();
      if (!mounted) return;
      setState(
        () => error = tooLarge
            ? limit['refusal'] as String? ?? 'That file is too large.'
            : why ?? 'PassBundle couldn\'t open that file.',
      );
      return;
    }
    setState(() {
      picked = {'path': path, 'name': fileName, 'size': size};
      error = null;
      if (name.text.trim().isEmpty) name.text = fileName;
    });
  }

  Future<void> save() async {
    final count = most.text.trim();
    final draft = <String, dynamic>{
      'kind': editing ? (send?['kind'] ?? 'text') : kind,
      'name': name.text,
      'notes': notes.text,
      'text': text.text,
      'hidden': hidden,
      'maxAccessCount': count.isEmpty ? null : int.tryParse(count) ?? 0,
      'deletionDate': deleteIn == _keep
          ? (send?['deletionDate'])
          : _after(deleteIn),
      'expirationDate': expireIn == _keep
          ? (send?['expirationDate'])
          : expireIn == _never
          ? null
          : _after(expireIn),
      'disabled': off,
      'hideEmail': hideEmail,
    };
    if (newFile) {
      await sendTheFile(draft);
      return;
    }
    setState(() {
      saving = true;
      error = null;
    });
    final answer = await whileBusy(
      () => Core.saveSend(widget.id, draft, password.text),
    );
    if (!mounted) return;
    password.clear();
    if (!answer.ok) {
      setState(() {
        saving = false;
        error = answer.error ?? 'Couldn\'t save it.';
      });
      return;
    }
    Navigator.of(context).pop((
      id: answer.body['id'] as String,
      synced: answer.body['synced'] != false,
    ));
  }

  /// Makes the file Send: the core reads the file, encrypts it here, and
  /// sends it, with how far it is shown below the file and Cancel. The
  /// editor stays open until it's done.
  Future<void> sendTheFile(Map<String, dynamic> draft) async {
    final file = picked;
    if (file == null) {
      setState(() => error = 'Choose a file to send.');
      return;
    }
    setState(() {
      saving = true;
      error = null;
    });
    final answer = await sendFile(
      widget.core,
      draft,
      file['path'] as String,
      file['name'] as String? ?? '',
      password.text,
    );
    if (!mounted) return;
    password.clear();
    setState(() {
      saving = false;
      // A phone's copy is deleted once the upload ends, however it ends.
      if (isPhone) picked = null;
      if (!answer.ok && answer.body['canceled'] != true) {
        error = answer.error ?? 'Couldn\'t send that file.';
      }
    });
    if (!answer.ok) return;
    Navigator.of(context).pop((
      id: answer.body['id'] as String,
      synced: answer.body['synced'] != false,
    ));
  }

  Future<void> removePassword() async {
    final id = widget.id;
    if (id == null) return;
    final messenger = ScaffoldMessenger.of(context);
    final answer = await whileBusy(() => Core.removeSendPassword(id));
    if (!mounted) return;
    if (!answer.ok) {
      setState(() => error = answer.error ?? 'Couldn\'t remove the password.');
      return;
    }
    setState(() => send = widget.core.send(id) ?? send);
    _say(messenger, 'Removed the password. Anyone with the link can open it.');
  }

  DropdownButtonFormField<Duration> _dates({
    required Key key,
    required String label,
    required Duration value,
    String? keep,
    String? never,
    required ValueChanged<Duration> onChanged,
  }) => DropdownButtonFormField<Duration>(
    key: key,
    initialValue: value,
    decoration: InputDecoration(labelText: label),
    items: [
      if (keep != null) DropdownMenuItem(value: _keep, child: Text(keep)),
      if (never != null) DropdownMenuItem(value: _never, child: Text(never)),
      for (final (words, d) in _presets)
        DropdownMenuItem(value: d, child: Text('In $words')),
    ],
    onChanged: (d) {
      if (d != null) onChanged(d);
    },
  );

  /// A new file Send's file: which, how large, and the most a file Send
  /// takes, said before one is chosen; while it goes, how far it is, and
  /// Cancel.
  Widget fileToSend(BuildContext context) {
    final c = context.colors;
    final file = picked;
    final size = file?['size'] as num?;
    return ValueListenableBuilder<Uploading?>(
      valueListenable: uploading,
      builder: (context, upload, _) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          if (file != null)
            Text(
              size == null
                  ? file['name'] as String
                  : '${file['name']} · ${fileSize(size)}',
              key: const ValueKey('send-file'),
              style: const TextStyle(fontSize: 16),
              overflow: TextOverflow.ellipsis,
            ),
          if (saving && upload != null && upload.isSend) ...[
            const SizedBox(height: 8),
            LinearProgressIndicator(
              key: const ValueKey('send-upload-progress'),
              value: upload.canceling ? null : upload.fraction,
              borderRadius: BorderRadius.circular(2),
            ),
            Row(
              children: [
                Expanded(
                  child: Text(
                    uploadWords(upload),
                    style: TextStyle(color: c.muted, fontSize: 13),
                  ),
                ),
                TextButton(
                  onPressed: upload.canceling
                      ? null
                      : () => cancelUpload(widget.core),
                  child: const Text('Cancel'),
                ),
              ],
            ),
          ] else
            Align(
              alignment: Alignment.centerLeft,
              child: TextButton.icon(
                key: const ValueKey('send-choose-file'),
                onPressed: saving ? null : chooseFile,
                icon: Icon(
                  Icons.attach_file_rounded,
                  size: 18,
                  color: Theme.of(context).colorScheme.primary,
                ),
                label: Text(
                  file == null ? 'Choose a file' : 'Choose another file',
                ),
              ),
            ),
          Text(
            'Up to $limitWords. It\'s encrypted on this device before it\'s sent.',
            style: TextStyle(color: c.muted, fontSize: 13),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final isFile = editing && send?['kind'] == 'file';
    final hasPassword = send?['hasPassword'] == true;
    final showHideEmail = !widget.noHidingEmail || hideEmail;
    // While a file goes, the editor stays: Cancel stops it.
    return PopScope(
      canPop: !saving,
      child: Scaffold(
        appBar: AppBar(
          title: Text(editing ? 'Edit Send' : 'New Send'),
          automaticallyImplyLeading: isPhone,
          leading: isPhone
              ? null
              : IconButton(
                  tooltip: 'Close',
                  icon: const Icon(Icons.close_rounded),
                  onPressed: saving ? null : () => Navigator.of(context).pop(),
                ),
          actions: [
            Padding(
              padding: const EdgeInsets.only(right: 12),
              child: WorkingButton(
                label: 'Save',
                working: saving,
                onPressed: save,
              ),
            ),
          ],
        ),
        body: ListView(
          padding: const EdgeInsets.fromLTRB(20, 8, 20, 28),
          children: [
            if (error case final e?)
              Padding(
                padding: const EdgeInsets.only(bottom: 12),
                child: Text(
                  e,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
              ),
            if (!editing) ...[
              SegmentedButton<String>(
                key: const ValueKey('send-kind'),
                showSelectedIcon: false,
                segments: const [
                  ButtonSegment(value: 'text', label: Text('Text')),
                  ButtonSegment(value: 'file', label: Text('File')),
                ],
                selected: {kind},
                onSelectionChanged: saving
                    ? null
                    : (chosen) => setState(() {
                        kind = chosen.first;
                        error = null;
                      }),
              ),
              const SizedBox(height: 14),
            ],
            TextField(
              key: const ValueKey('send-name'),
              controller: name,
              autofocus: !editing,
              decoration: const InputDecoration(labelText: 'Name'),
            ),
            const SizedBox(height: 14),
            if (isFile)
              Text(
                'File: ${send?['fileName'] ?? ''}. A Send\'s file can\'t change.',
                style: TextStyle(color: c.muted),
              )
            else if (newFile)
              fileToSend(context)
            else ...[
              TextField(
                key: const ValueKey('send-text'),
                controller: text,
                minLines: 3,
                maxLines: 8,
                decoration: const InputDecoration(labelText: 'Text to share'),
              ),
              SwitchListTile(
                contentPadding: EdgeInsets.zero,
                value: hidden,
                onChanged: (on) => setState(() => hidden = on),
                title: const Text('Hide the text until it\'s shown'),
                subtitle: const Text(
                  'Whoever opens it presses a button to see it.',
                ),
              ),
            ],
            const SizedBox(height: 8),
            _dates(
              key: const ValueKey('send-deletion'),
              label: 'Deleted',
              value: deleteIn,
              keep: editing
                  ? 'On ${sendDate(send?['deletionDate'] as String?)}'
                  : null,
              onChanged: (d) => setState(() => deleteIn = d),
            ),
            const SizedBox(height: 14),
            _dates(
              key: const ValueKey('send-expiration'),
              label: 'Stops opening',
              value: expireIn,
              keep: send?['expirationDate'] == null
                  ? null
                  : 'On ${sendDate(send?['expirationDate'] as String?)}',
              never: 'When it\'s deleted',
              onChanged: (d) => setState(() => expireIn = d),
            ),
            const SizedBox(height: 14),
            TextField(
              key: const ValueKey('send-most'),
              controller: most,
              keyboardType: TextInputType.number,
              inputFormatters: [FilteringTextInputFormatter.digitsOnly],
              decoration: const InputDecoration(
                labelText: 'Most times it can be opened (optional)',
              ),
            ),
            const SizedBox(height: 14),
            TextField(
              key: const ValueKey('send-password'),
              controller: password,
              obscureText: true,
              decoration: InputDecoration(
                labelText: hasPassword
                    ? 'New password (optional)'
                    : 'Password (optional)',
                helperText: hasPassword
                    ? 'It has a password. Leave this empty to keep it.'
                    : 'Whoever opens it types this first. Share it another way than the link.',
                helperMaxLines: 2,
              ),
            ),
            if (hasPassword)
              Align(
                alignment: Alignment.centerLeft,
                child: TextButton(
                  onPressed: removePassword,
                  child: const Text('Remove password'),
                ),
              ),
            const SizedBox(height: 14),
            TextField(
              key: const ValueKey('send-notes'),
              controller: notes,
              minLines: 1,
              maxLines: 4,
              decoration: const InputDecoration(
                labelText: 'Private note',
                helperText: 'Only you see this.',
              ),
            ),
            if (showHideEmail)
              SwitchListTile(
                contentPadding: EdgeInsets.zero,
                value: hideEmail,
                onChanged: (on) => setState(() => hideEmail = on),
                title: const Text('Hide my email from whoever opens it'),
                subtitle: widget.noHidingEmail
                    ? const Text(
                        'An organization you\'re in doesn\'t allow this. Turn it off to save.',
                      )
                    : null,
              ),
            SwitchListTile(
              contentPadding: EdgeInsets.zero,
              value: off,
              onChanged: (on) => setState(() => off = on),
              title: const Text('Turn this Send off'),
              subtitle: const Text('No one can open it while it\'s off.'),
            ),
          ],
        ),
      ),
    );
  }
}
