// Exporting the person's own items to a file, and importing a file (DESIGN.md
// in the meta repo, "Archive, export, and import in the apps"; the core's
// export.rs, import.rs, and app/transfer.rs).
//
// Export offers the web vault's three files, asks for the master password
// (never a PIN or biometrics), then where the file goes: the computer's own
// save dialog, or on a phone the system's picker, which the file is handed
// to and then deleted from the app's folder. The core writes the file; its
// contents never pass through here.
//
// Import picks a format, then the file (the attachments' picker), shows what
// came in and what didn't before anything is saved, and where it goes:
// Personal, or a shared folder the person may add to when an organization
// keeps items out of Personal. Formats the apps don't read yet go to the web
// vault's import.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

/// The web vault's import, for the formats the app doesn't read yet.
const webVaultImportPath = '/#/tools/import';

/// Exports the person's own items, in a dialog: what to say once it's
/// done, or null if nothing was exported.
Future<String?> showExport(BuildContext context) => showDialog<String>(
  context: context,
  barrierDismissible: false,
  builder: (_) => const ExportDialog(),
);

/// Counts the imports that brought something in: the vault reads itself
/// again (vault.dart), wherever the import was started from.
final vaultChanged = ValueNotifier<int>(0);

/// Imports a file, in a dialog: [format] chosen to begin with (a browser's,
/// from the browser import's "Import with its own Export"), and [server]
/// for the formats the web vault imports. True if anything came in.
Future<bool> showImport(
  BuildContext context, {
  String? format,
  String? server,
}) async {
  final came =
      await showDialog<bool>(
        context: context,
        barrierDismissible: false,
        builder: (_) => ImportDialog(format: format, server: server),
      ) ==
      true;
  if (came) vaultChanged.value++;
  return came;
}

/// "20261116093012": now, where this device is, as the web vault names its
/// exports.
String exportStamp(DateTime now) {
  String two(int n) => n.toString().padLeft(2, '0');
  return '${now.year}${two(now.month)}${two(now.day)}'
      '${two(now.hour)}${two(now.minute)}${two(now.second)}';
}

/// The export's file name, as the web vault names it, with PassBundle's name.
String exportFileName(String format, DateTime now) => switch (format) {
  'csv' => 'passbundle_export_${exportStamp(now)}.csv',
  'encrypted_json' => 'passbundle_encrypted_export_${exportStamp(now)}.json',
  _ => 'passbundle_export_${exportStamp(now)}.json',
};

/// "1 item", "3 items".
String _items(int n) => n == 1 ? '1 item' : '$n items';

/// How to delete a file that holds passwords in plain text, on this device.
String deleteSteps(String name) {
  if (isPhone) return 'Delete $name from where you saved it.';
  if (Platform.isMacOS) return 'Move $name to the Trash, then empty the Trash.';
  if (Platform.isWindows) {
    return 'Select $name and press Shift+Delete, so it skips the Recycle Bin.';
  }
  return 'Delete $name, and empty the trash if it goes there.';
}

class ExportDialog extends StatefulWidget {
  const ExportDialog({super.key});

  @override
  State<ExportDialog> createState() => _ExportDialogState();
}

class _ExportDialogState extends State<ExportDialog> {
  late final Answer info = Core().exportInfo();
  String format = 'json';
  final filePassword = TextEditingController();
  final again = TextEditingController();
  final master = TextEditingController();
  String? fileError, masterError, error;
  bool working = false;

  @override
  void dispose() {
    filePassword.dispose();
    again.dispose();
    master.dispose();
    super.dispose();
  }

  bool get protected => format == 'encrypted_json';

  int get count =>
      ((format == 'csv' ? info.body['csvItems'] : info.body['items']) as num? ??
              0)
          .toInt();

  Future<void> export() async {
    setState(() {
      fileError = masterError = error = null;
    });
    if (protected && filePassword.text.isEmpty) {
      setState(() => fileError = 'Choose a password for the file.');
      return;
    }
    if (protected && filePassword.text != again.text) {
      setState(() => fileError = 'The two passwords aren\'t the same.');
      return;
    }
    if (master.text.isEmpty) {
      setState(() => masterError = 'Enter your master password to export.');
      return;
    }
    setState(() => working = true);
    final confirmed = await whileBusy(() => Core.confirmExport(master.text));
    if (!mounted) return;
    if (!confirmed.ok) {
      setState(() {
        working = false;
        masterError = confirmed.error ?? 'That master password isn\'t right.';
      });
      return;
    }
    master.clear();
    final name = exportFileName(format, DateTime.now());
    String? chosen;
    try {
      if (isPhone) {
        final folder = await Core.exportFolder();
        chosen = folder == null ? null : '$folder/$name';
      } else {
        chosen = await Core.chooseSaveLocation(name);
      }
    } on PlatformException catch (e) {
      if (!mounted) return;
      setState(() {
        working = false;
        error = e.message ?? 'PassBundle couldn\'t open the save dialog.';
      });
      return;
    }
    if (!mounted) return;
    if (chosen == null) {
      // Gave up choosing where: nothing written.
      setState(() => working = false);
      return;
    }
    final path = chosen;
    final secret = protected ? filePassword.text : '';
    final answer = await whileBusy(() => Core.export(format, path, secret));
    if (!mounted) return;
    if (!answer.ok) {
      setState(() {
        working = false;
        error = answer.error ?? 'Couldn\'t export.';
      });
      return;
    }
    final items = (answer.body['items'] as num? ?? 0).toInt();
    final leftOut = (answer.body['leftOut'] as num? ?? 0).toInt();
    var saved = true;
    if (isPhone) {
      saved = await Core.handOnFile(path, name);
      if (!mounted) return;
    }
    filePassword.clear();
    again.clear();
    final where = isPhone ? '' : ' to ${path.split(RegExp(r'[/\\]')).last}';
    final notIn = leftOut == 0
        ? ''
        : format == 'csv'
        ? ' ${_items(leftOut)} ${leftOut == 1 ? 'isn\'t' : 'aren\'t'} in it: a .csv holds logins and notes only.'
        : ' ${_items(leftOut)} that ${leftOut == 1 ? 'doesn\'t' : 'don\'t'} open on this device ${leftOut == 1 ? 'isn\'t' : 'aren\'t'} in it.';
    Navigator.of(context).pop(
      saved
          ? 'Exported ${_items(items)}$where.$notIn'
          : 'The export wasn\'t saved anywhere, so PassBundle deleted it.',
    );
  }

  @override
  Widget build(BuildContext context) {
    final muted = TextStyle(color: context.colors.muted, height: 1.35);
    final forbiddenBy = info.body['forbiddenBy'] as String?;
    return _Frame(
      title: 'Export',
      actions: [
        TextButton(
          onPressed: working ? null : () => Navigator.of(context).pop(),
          child: Text(forbiddenBy != null || !info.ok ? 'Close' : 'Cancel'),
        ),
        if (forbiddenBy == null && info.ok) ...[
          const SizedBox(width: 8),
          WorkingButton(
            key: const ValueKey('export-go'),
            label: 'Export',
            working: working,
            onPressed: export,
          ),
        ],
      ],
      children: [
        if (!info.ok)
          Text(info.error ?? 'Couldn\'t export.', style: muted)
        else if (forbiddenBy != null)
          Text(
            '$forbiddenBy doesn\'t let you export your own items.',
            style: muted,
          )
        else ...[
          Text(
            'Exports the ${_items(count)} in Personal.'
            '${info.body['inOrganization'] == true ? ' An organization\'s items are exported in the web vault.' : ''}',
            style: muted,
          ),
          RadioGroup<String>(
            groupValue: format,
            onChanged: (f) => setState(() {
              format = f ?? format;
              fileError = null;
            }),
            child: SettingsGroup(
              title: 'File format',
              children: const [
                RadioListTile<String>(
                  key: ValueKey('format-json'),
                  value: 'json',
                  title: Text('.json'),
                  subtitle: Text('Every kind of item, with its folders'),
                ),
                RadioListTile<String>(
                  key: ValueKey('format-csv'),
                  value: 'csv',
                  title: Text('.csv'),
                  subtitle: Text('Logins and notes only'),
                ),
                RadioListTile<String>(
                  key: ValueKey('format-encrypted_json'),
                  value: 'encrypted_json',
                  title: Text('Password-protected .json'),
                  subtitle: Text(
                    'Encrypted with a password you choose. PassBundle and the web vault open it.',
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 12),
          if (protected) ...[
            TextField(
              key: const ValueKey('file-password'),
              controller: filePassword,
              obscureText: true,
              enableSuggestions: false,
              autocorrect: false,
              decoration: const InputDecoration(labelText: 'File password'),
            ),
            const SizedBox(height: 10),
            TextField(
              key: const ValueKey('file-password-again'),
              controller: again,
              obscureText: true,
              enableSuggestions: false,
              autocorrect: false,
              decoration: InputDecoration(
                labelText: 'File password again',
                errorText: fileError,
                helperText: 'Keep this password: without it, the file can\'t be opened.',
                helperMaxLines: 2,
              ),
            ),
          ] else
            Text(
              'This file holds your passwords in plain text. Keep it somewhere safe, and delete it once you\'re done with it.',
              style: muted,
            ),
          const SizedBox(height: 14),
          TextField(
            key: const ValueKey('export-master-password'),
            controller: master,
            obscureText: true,
            enableSuggestions: false,
            autocorrect: false,
            decoration: InputDecoration(
              labelText: 'Master password',
              errorText: masterError,
            ),
            onSubmitted: (_) {
              if (!working) export();
            },
          ),
          if (error case final error?)
            Padding(
              padding: const EdgeInsets.only(top: 10),
              child: Text(
                error,
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
            ),
        ],
      ],
    );
  }
}

/// A format the app imports: its id for the core, its name, and the group
/// it's listed in.
class ImportFormat {
  const ImportFormat(this.id, this.name, {this.group = 'PassBundle'});

  final String id;
  final String name;
  final String group;
}

/// The formats, in the order offered.
const importFormats = [
  ImportFormat('json', 'PassBundle or the web vault (.json)'),
  ImportFormat('csv', 'PassBundle or the web vault (.csv)'),
  ImportFormat('chrome', 'Google Chrome', group: 'Browsers'),
  ImportFormat('edge', 'Microsoft Edge', group: 'Browsers'),
  ImportFormat('brave', 'Brave', group: 'Browsers'),
  ImportFormat('opera', 'Opera', group: 'Browsers'),
  ImportFormat('vivaldi', 'Vivaldi', group: 'Browsers'),
  ImportFormat('firefox', 'Firefox', group: 'Browsers'),
  ImportFormat('safari', 'Safari or the Passwords app', group: 'Browsers'),
  ImportFormat(
    '1password-csv',
    '1Password 8 (.csv)',
    group: 'Password managers',
  ),
];

/// A browser's export, step by step, as the web vault's guided import has
/// them, for this computer.
List<String> exportSteps(String format) {
  final mac = !isPhone && Platform.isMacOS;
  final windows = !isPhone && Platform.isWindows;
  final confirm = mac
      ? 'Confirm with Touch ID or your Mac\'s password.'
      : windows
      ? 'Confirm with Windows Hello or your computer\'s password.'
      : 'Confirm with your computer\'s password if it asks.';
  return switch (format) {
    'chrome' || 'chromium' => [
      'In Chrome, type chrome://password-manager/settings in the address bar.',
      'Next to Export passwords, choose Download file.',
      confirm,
      'Save the file, Chrome Passwords.csv, somewhere you\'ll find it, such as Downloads.',
    ],
    'edge' => [
      'In Edge, open Settings, then Passwords (or type edge://wallet/passwords in the address bar).',
      'Choose More actions (…) next to Saved passwords, then Export passwords.',
      'Choose Export passwords again. $confirm',
      'Save the file, Microsoft Edge Passwords.csv, somewhere you\'ll find it, such as Downloads.',
    ],
    'brave' => [
      'In Brave, type brave://password-manager/settings in the address bar.',
      'Next to Export passwords, choose Download file.',
      confirm,
      'Save the file, Brave Passwords.csv, somewhere you\'ll find it, such as Downloads.',
    ],
    'opera' => [
      'In Opera\'s settings, find its passwords, then choose Export passwords.',
      confirm,
      'Save the file somewhere you\'ll find it, such as Downloads.',
    ],
    'vivaldi' => [
      'In Vivaldi\'s settings, under Privacy and Security, find its passwords, then choose Export.',
      confirm,
      'Save the file somewhere you\'ll find it, such as Downloads.',
    ],
    'firefox' => [
      'In Firefox, open the menu (☰), then Passwords (or type about:logins in the address bar).',
      'Choose the menu (…) at the top right, then Export passwords.',
      'Choose Continue with export. $confirm',
      'Save the file, logins.csv, somewhere you\'ll find it, such as Downloads.',
    ],
    'safari' => [
      'Open the Passwords app (in Applications). On macOS 14 or earlier, open Safari\'s Settings, then Passwords.',
      'Choose File, then Export All Passwords to File (in Safari: the … button, then Export All Passwords).',
      'Choose Continue. Confirm with Touch ID or your Mac\'s password.',
      'Save the file, Passwords.csv, somewhere you\'ll find it, such as Downloads.',
    ],
    _ => const [],
  };
}

enum _Stage { choosing, reading, password, summary, saving, done }

class ImportDialog extends StatefulWidget {
  const ImportDialog({super.key, this.format, this.server});

  final String? format;
  final String? server;

  @override
  State<ImportDialog> createState() => _ImportDialogState();
}

class _ImportDialogState extends State<ImportDialog> {
  late String format = widget.format == 'chromium'
      ? 'chrome'
      : widget.format ?? 'json';
  _Stage stage = _Stage.choosing;
  String? error;
  bool wrongPassword = false;
  final password = TextEditingController();
  Map<String, dynamic> summary = const {};
  String? path, fileName;
  String? to;
  int added = 0;

  @override
  void dispose() {
    password.dispose();
    super.dispose();
  }

  ImportFormat get chosen => importFormats.firstWhere(
    (f) => f.id == format,
    orElse: () => importFormats.first,
  );

  Future<void> choose() async {
    setState(() => error = null);
    // The largest file the core imports (core/src/app/transfer.rs).
    Map<String, dynamic>? picked;
    try {
      picked = await Core.pickFile(100 * 1024 * 1024, import: true);
    } on PlatformException catch (e) {
      if (mounted) {
        setState(() => error = e.message ?? 'Couldn\'t open the file picker.');
      }
      return;
    }
    if (!mounted || picked == null) return;
    if (picked['tooLarge'] == true) {
      setState(
        () => error = 'That file is larger than PassBundle imports (100 MB).',
      );
      return;
    }
    if (picked['error'] case final String words) {
      setState(() => error = words);
      return;
    }
    path = picked['path'] as String?;
    fileName = picked['name'] as String?;
    await read();
  }

  Future<void> read([String filePassword = '']) async {
    final at = path;
    if (at == null) return;
    setState(() => stage = _Stage.reading);
    final answer = await whileBusy(
      () => Core.importRead(format, at, filePassword),
    );
    if (!mounted) return;
    if (!answer.ok) {
      setState(() {
        stage = _Stage.choosing;
        error = answer.error;
      });
      await Core.forgetPicked();
      return;
    }
    switch (answer.body['read']) {
      case 'needsPassword':
        setState(() => stage = _Stage.password);
      case 'wrongPassword':
        setState(() {
          stage = _Stage.password;
          wrongPassword = true;
        });
      default:
        password.clear();
        final personal = answer.body['personal'] == true;
        final folders = ((answer.body['sharedFolders'] as List?) ?? const [])
            .cast<Map<String, dynamic>>();
        setState(() {
          summary = answer.body;
          wrongPassword = false;
          to = personal
              ? 'personal'
              : folders.isEmpty
              ? null
              : 'folder:${folders.first['id']}';
          stage = _Stage.summary;
        });
    }
  }

  Future<void> save() async {
    final destination = to;
    if (destination == null) return;
    setState(() {
      stage = _Stage.saving;
      error = null;
    });
    final answer = await whileBusy(() => Core.importSave(destination));
    if (!mounted) return;
    if (!answer.ok) {
      final saved = (answer.body['saved'] as num? ?? 0).toInt();
      setState(() {
        stage = _Stage.summary;
        error = saved == 0
            ? answer.error
            : '${_items(saved)} came in before this: ${answer.error} Import again brings in the rest.';
        if (saved > 0) {
          summary = {
            ...summary,
            'items': ((summary['items'] as num? ?? 0) - saved).toInt(),
          };
        }
      });
      return;
    }
    added = (answer.body['added'] as num? ?? 0).toInt();
    await Core.forgetPicked();
    if (mounted) setState(() => stage = _Stage.done);
  }

  void cancel() {
    if (stage == _Stage.summary || stage == _Stage.password) {
      Core().importCancel();
    }
    Core.forgetPicked();
    Navigator.of(context).pop(added > 0);
  }

  Future<void> openWebVault() async {
    final server = widget.server;
    final opened =
        server != null && await Core.openWebsite('$server$webVaultImportPath');
    if (!opened && mounted) {
      setState(() => error = 'Couldn\'t open the web vault.');
    }
  }

  @override
  Widget build(BuildContext context) => _Frame(
    title: switch (stage) {
      _Stage.password => 'This file has a password',
      _Stage.done => 'Imported',
      _ => 'Import',
    },
    actions: actions(context),
    children: [
      switch (stage) {
        _Stage.choosing => choosing(context),
        _Stage.reading || _Stage.saving => const Padding(
          padding: EdgeInsets.symmetric(vertical: 48),
          child: Center(child: CircularProgressIndicator()),
        ),
        _Stage.password => passwordStage(context),
        _Stage.summary => summaryStage(context),
        _Stage.done => doneStage(context),
      },
      if (error case final error?)
        Padding(
          padding: const EdgeInsets.only(top: 12),
          child: Text(
            error,
            key: const ValueKey('import-error'),
            style: TextStyle(color: Theme.of(context).colorScheme.error),
          ),
        ),
    ],
  );

  List<Widget> actions(BuildContext context) => switch (stage) {
    _Stage.choosing => [
      TextButton(onPressed: cancel, child: const Text('Cancel')),
      const SizedBox(width: 8),
      FilledButton(
        key: const ValueKey('import-choose-file'),
        onPressed: choose,
        child: const Text('Choose file'),
      ),
    ],
    _Stage.reading || _Stage.saving => const [],
    _Stage.password => [
      TextButton(onPressed: cancel, child: const Text('Cancel')),
      const SizedBox(width: 8),
      FilledButton(
        onPressed: () => read(password.text),
        child: const Text('Continue'),
      ),
    ],
    _Stage.summary => [
      TextButton(onPressed: cancel, child: const Text('Cancel')),
      const SizedBox(width: 8),
      FilledButton(
        key: const ValueKey('import-go'),
        onPressed: to == null || (summary['items'] as num? ?? 0) == 0
            ? null
            : save,
        child: const Text('Import'),
      ),
    ],
    _Stage.done => [
      FilledButton(
        onPressed: () => Navigator.of(context).pop(added > 0),
        child: const Text('Done'),
      ),
    ],
  };

  Widget choosing(BuildContext context) {
    final muted = TextStyle(color: context.colors.muted, height: 1.35);
    final steps = exportSteps(format);
    final groups = <String, List<ImportFormat>>{};
    for (final f in importFormats) {
      groups.putIfAbsent(f.group, () => []).add(f);
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          'Choose what the file is from. PassBundle shows what\'s in it before anything is saved.',
          style: muted,
        ),
        RadioGroup<String>(
          groupValue: format,
          onChanged: (f) => setState(() {
            format = f ?? format;
            error = null;
          }),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final MapEntry(key: group, value: formats) in groups.entries)
                SettingsGroup(
                  title: group,
                  children: [
                    for (final f in formats)
                      RadioListTile<String>(
                        key: ValueKey('import-format-${f.id}'),
                        value: f.id,
                        title: Text(f.name),
                      ),
                    if (group == 'Password managers' && widget.server != null)
                      ListTile(
                        key: const ValueKey('import-in-web-vault'),
                        title: const Text('Another format, in the web vault'),
                        subtitle: const Text(
                          'LastPass, 1Password\'s .1pux, KeePass, Dashlane, and more',
                        ),
                        trailing: const Icon(
                          Icons.open_in_new_rounded,
                          size: 20,
                        ),
                        onTap: openWebVault,
                      ),
                  ],
                ),
            ],
          ),
        ),
        if (steps.isNotEmpty)
          SettingsGroup(
            title: 'Export from ${chosen.name}',
            footer: 'Then choose the file here. It has every password in plain text, so delete it once they\'re in.',
            children: [
              for (final (i, step) in steps.indexed)
                ListTile(
                  dense: true,
                  leading: Text('${i + 1}.', style: muted),
                  minLeadingWidth: 16,
                  title: Text(step),
                ),
            ],
          ),
      ],
    );
  }

  Widget passwordStage(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    mainAxisSize: MainAxisSize.min,
    children: [
      Text(
        '${fileName ?? 'This file'} is password-protected. Type the password it was exported with.',
        style: TextStyle(color: context.colors.muted, height: 1.35),
      ),
      const SizedBox(height: 16),
      TextField(
        key: const ValueKey('import-file-password'),
        controller: password,
        obscureText: true,
        autofocus: true,
        enableSuggestions: false,
        autocorrect: false,
        decoration: InputDecoration(
          labelText: 'File password',
          errorText: wrongPassword
              ? 'That isn\'t this file\'s password.'
              : null,
        ),
        onSubmitted: (_) => read(password.text),
      ),
    ],
  );

  /// What came in and what didn't, and where it goes.
  Widget summaryStage(BuildContext context) {
    final muted = TextStyle(color: context.colors.muted, height: 1.35);
    String plural(int n, String one, String many) => n == 1 ? one : many;
    int n(String key) => (summary[key] as num? ?? 0).toInt();
    final kinds = (summary['kinds'] as Map?)?.cast<String, dynamic>() ?? {};
    int kind(String k) => (kinds[k] as num? ?? 0).toInt();
    final parts = [
      for (final (k, one, many) in [
        ('login', 'login', 'logins'),
        ('note', 'note', 'notes'),
        ('card', 'card', 'cards'),
        ('identity', 'identity', 'identities'),
        ('sshKey', 'SSH key', 'SSH keys'),
        ('other', 'other item', 'other items'),
      ])
        if (kind(k) > 0) '${kind(k)} ${plural(kind(k), one, many)}',
    ];
    final items = n('items');
    final folders = ((summary['sharedFolders'] as List?) ?? const [])
        .cast<Map<String, dynamic>>();
    final personal = summary['personal'] == true;
    final shared = to?.startsWith('folder:') ?? false;
    final checkFolder = summary['checkFolder'] as String?;
    final lines = <String>[
      if (items == 0)
        'Nothing in this file comes in.'
      else
        '${_items(items)} come${items == 1 ? 's' : ''} in: ${parts.join(', ')}.',
      if (n('folders') > 0 && !shared)
        'With ${n('folders')} ${plural(n('folders'), 'folder', 'folders')}; one you have already by the same name is used.',
      if (n('folders') > 0 && shared) 'Everything goes in the one shared folder; the file\'s folders aren\'t kept.',
      if (n('archived') > 0)
        '${n('archived')} ${plural(n('archived'), 'is', 'are')} archived, and stay${n('archived') == 1 ? 's' : ''} in Archive.',
      if (n('toCheck') > 0 && checkFolder != null && !shared)
        '${n('toCheck')} ${plural(n('toCheck'), 'has', 'have')} a different password from a login you already have, so ${plural(n('toCheck'), 'it goes', 'they go')} in "$checkFolder" for you to compare.',
      if (n('toCheck') > 0 && shared)
        '${n('toCheck')} ${plural(n('toCheck'), 'has', 'have')} a different password from a login you already have.',
      for (final left
          in ((summary['leftOut'] as List?) ?? const [])
              .cast<Map<String, dynamic>>())
        switch ((left['why'], (left['count'] as num? ?? 0).toInt())) {
          ('noPassword', final c) =>
            '$c saved ${plural(c, 'sign-in has', 'sign-ins have')} no password, so ${plural(c, 'it\'s', 'they\'re')} skipped.',
          ('alreadyThere', final c) =>
            '$c ${plural(c, 'is', 'are')} already in your vault, or twice in the file, so ${plural(c, 'it\'s', 'they\'re')} left out.',
          ('unknownKind', final c) =>
            '$c ${plural(c, 'is a kind', 'are kinds')} of item PassBundle doesn\'t know, so ${plural(c, 'it\'s', 'they\'re')} left out.',
          ('firefoxAccount', _) => 'Firefox\'s own account entry is left out.',
          (_, final c) => '$c ${plural(c, 'item is', 'items are')} left out.',
        },
      if (n('renamed') > 0)
        '${n('renamed')} ${plural(n('renamed'), 'field is', 'fields are')} named like PassBundle\'s own, so ${plural(n('renamed'), 'it comes', 'they come')} in as ${plural(n('renamed'), 'a field', 'fields')} named "Imported …".',
    ];
    final keepsOut = summary['keepsItemsOut'] as String?;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final line in lines)
          Padding(
            padding: const EdgeInsets.only(bottom: 6),
            child: Text(line, style: muted),
          ),
        if (!personal && folders.isEmpty)
          Text(
            '${keepsOut ?? 'Your organization'} keeps items out of Personal, and you have no shared folder you can add to. Ask an admin for one, then import the file into it.',
            key: const ValueKey('import-nowhere'),
            style: muted,
          )
        else
          SettingsGroup(
            title: 'Import into',
            footer: !personal
                ? '${keepsOut ?? 'Your organization'} keeps items out of Personal.'
                : null,
            children: [
              RadioGroup<String>(
                groupValue: to,
                onChanged: (t) => setState(() => to = t),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (personal)
                      const RadioListTile<String>(
                        key: ValueKey('import-to-personal'),
                        value: 'personal',
                        title: Text('Personal'),
                      ),
                    for (final f in folders)
                      RadioListTile<String>(
                        key: ValueKey('import-to-${f['id']}'),
                        value: 'folder:${f['id']}',
                        title: Text('${f['organization']} › ${f['name']}'),
                      ),
                  ],
                ),
              ),
            ],
          ),
      ],
    );
  }

  Widget doneStage(BuildContext context) {
    final muted = TextStyle(color: context.colors.muted, height: 1.35);
    final plain = summary['protected'] != true;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          '${_items(added)} came in.',
          key: const ValueKey('import-done'),
          style: muted,
        ),
        if (plain) ...[
          const SizedBox(height: 8),
          Text(
            '${fileName ?? 'The file'} has every password in plain text. ${deleteSteps(fileName ?? 'it')}',
            style: muted,
          ),
        ],
      ],
    );
  }
}

/// A dialog's frame: its title, what's in it (scrolling past its most), and
/// its buttons.
class _Frame extends StatelessWidget {
  const _Frame({
    required this.title,
    required this.children,
    required this.actions,
  });

  final String title;
  final List<Widget> children;
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) => Dialog(
    backgroundColor: context.colors.dialogGround,
    clipBehavior: Clip.antiAlias,
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 520, maxHeight: 760),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(24, 22, 24, 0),
            child: Text(
              title,
              style: Theme.of(context).textTheme.headlineSmall,
            ),
          ),
          Flexible(
            child: SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(24, 12, 24, 8),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: children,
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(24, 4, 16, 12),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: actions,
            ),
          ),
        ],
      ),
    ),
  );
}
