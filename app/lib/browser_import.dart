// Importing the passwords saved in this computer's browsers (DESIGN.md in
// the meta repo, "Automatic import"; the core's browser_import.rs). On a
// computer, after unlocking, it's offered once by itself, and it's always in
// the menu ("Import from browsers").
//
// The person sees every browser profile here, which is which (its name, the
// account it's signed in to, whether it's the browser's default), and which
// are chosen and why; personal ones never are. Nothing is read until they
// choose Import, each time. A Firefox profile with a Primary Password asks
// for it, used once and never kept. A browser whose passwords only it can
// read (Chrome on Windows, Safari) goes to its own Export, said in plain
// words, then to importing that file here (transfer.dart), with that
// browser chosen and its export's steps. Afterwards, each
// profile read has its own password saving turned off (what it saved
// stays); an open browser is asked to close first.
import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'transfer.dart';
import 'widgets.dart';

/// After unlocking, on a computer: the import, offered once per account
/// here, when there's a browser profile to import from (the core says).
Future<void> offerBrowserImport(
  BuildContext context, {
  String? email,
  String? server,
}) async {
  if (isPhone) return;
  final offer = await Core.browserImportOffer();
  if (!offer || !context.mounted) return;
  await showBrowserImport(context, email: email, server: server);
}

Future<void> showBrowserImport(
  BuildContext context, {
  String? email,
  String? server,
}) => showDialog<void>(
  context: context,
  barrierDismissible: false,
  builder: (_) => BrowserImportDialog(email: email, server: server),
);

/// A browser by the name people call it: "Chrome", "Edge", "Firefox".
String browserOf(Map<String, dynamic> p) =>
    p['browserShort'] as String? ?? p['browserName'] as String? ?? 'Browser';

/// A profile by the name its browser shows: Firefox's own name for its
/// default profile ("default-release") reads as "Default".
String profileOf(Map<String, dynamic> p) {
  final name = p['name'] as String? ?? '';
  if (p['browser'] == 'firefox' &&
      RegExp(r'^default(-[a-z]+)?$').hasMatch(name)) {
    return 'Default';
  }
  return name;
}

/// A profile with its browser, one way everywhere: "Chrome: Work".
String profileName(Map<String, dynamic> p) => '${browserOf(p)}: ${profileOf(p)}';

enum _Stage { finding, choosing, importing, primaryPassword, done }

class BrowserImportDialog extends StatefulWidget {
  const BrowserImportDialog({super.key, this.email, this.server});

  final String? email;

  /// The web vault's address, for the formats it imports.
  final String? server;

  @override
  State<BrowserImportDialog> createState() => _BrowserImportDialogState();
}

class _BrowserImportDialogState extends State<BrowserImportDialog> {
  Map<String, dynamic> found = const {};
  final chosen = <String>{};
  _Stage stage = _Stage.finding;
  String? error;

  @override
  void initState() {
    super.initState();
    find();
  }

  /// The profiles here, and which the rules chose: nothing secret.
  Future<void> find() async {
    final answer = await Core.browserProfiles();
    if (!mounted) return;
    setState(() {
      found = answer.body;
      chosen
        ..clear()
        ..addAll([
          for (final p in profiles)
            if (p['chosen'] == true) p['id'] as String,
        ]);
      stage = _Stage.choosing;
    });
  }

  /// What the imports so far came to, added up.
  int added = 0, toCheck = 0, alreadyThere = 0, empty = 0, unreadable = 0;
  final checkFolders = <String>{};

  /// Each profile read, with how many it had; each that goes to its
  /// browser's own Export, with why.
  final read = <Map<String, dynamic>>[];
  final guided = <Map<String, dynamic>>[];

  /// Firefox profiles waiting for their Primary Password, the one asked
  /// for now first, and whether the last one given was wrong.
  final waiting = <Map<String, dynamic>>[];
  bool wrong = false;
  final primary = TextEditingController();

  /// Turning off each read profile's own password saving: its result.
  final turnedOff = <String, Map<String, dynamic>>{};

  List<Map<String, dynamic>> get profiles =>
      ((found['profiles'] as List?) ?? const []).cast<Map<String, dynamic>>();

  List<Map<String, dynamic>> get guidedBrowsers =>
      ((found['guided'] as List?) ?? const []).cast<Map<String, dynamic>>();

  @override
  void dispose() {
    primary.dispose();
    super.dispose();
  }

  Future<void> import(
    List<String> ids, [
    Map<String, String>? passwords,
  ]) async {
    setState(() {
      stage = _Stage.importing;
      error = null;
    });
    final answer = await whileBusy(
      () => Core.browserImport(ids, passwords ?? const {}),
    );
    if (!mounted) return;
    if (!answer.ok) {
      setState(() {
        error = answer.error;
        stage = waiting.isEmpty ? _Stage.choosing : _Stage.primaryPassword;
      });
      return;
    }
    final body = answer.body;
    int count(String key) => (body[key] as num?)?.toInt() ?? 0;
    added += count('added');
    toCheck += count('toCheck');
    alreadyThere += count('alreadyThere');
    empty += count('empty');
    unreadable += count('unreadable');
    checkFolders.addAll(((body['checkFolders'] as List?) ?? []).cast<String>());
    final results = ((body['profiles'] as List?) ?? [])
        .cast<Map<String, dynamic>>();
    wrong = false;
    waiting.removeWhere((w) => results.any((r) => r['id'] == w['id']));
    for (final result in results) {
      switch (result['outcome']) {
        case 'read':
          read.add(result);
        case 'needsPrimaryPassword':
          waiting.add(result);
        case 'wrongPrimaryPassword':
          waiting.insert(0, result);
          wrong = true;
        case 'guided':
          guided.add(result);
      }
    }
    // Then each profile read has its own password saving turned off.
    final readIds = [
      for (final r in results)
        if (r['outcome'] == 'read') r['id'] as String,
    ];
    if (readIds.isNotEmpty) await turnOff(readIds);
    if (!mounted) return;
    primary.clear();
    setState(
      () => stage = waiting.isEmpty ? _Stage.done : _Stage.primaryPassword,
    );
  }

  Future<void> turnOff(List<String> ids) async {
    final answer = await Core.browserTurnOff(ids);
    for (final p
        in ((answer.body['profiles'] as List?) ?? [])
            .cast<Map<String, dynamic>>()) {
      turnedOff[p['id'] as String] = p;
    }
    if (mounted) setState(() {});
  }

  /// The browser's own Export, then its file imported here (transfer.dart),
  /// with that browser chosen and its export's steps; the vault shows what
  /// came in.
  Future<void> importWithItsExport(String? browser) =>
      showImport(context, format: browser, server: widget.server);

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
            child: Text(switch (stage) {
              _Stage.primaryPassword => 'Firefox\'s Primary Password',
              // "Imported" only once something was; else the summary says why not.
              _Stage.done when added > 0 => 'Imported from your browsers',
              _ => 'Import from your browsers',
            }, style: Theme.of(context).textTheme.headlineSmall),
          ),
          Flexible(
            child: SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(20, 8, 20, 8),
              child: switch (stage) {
                _Stage.choosing => choosing(context),
                _Stage.finding || _Stage.importing => const Padding(
                  padding: EdgeInsets.symmetric(vertical: 48),
                  child: Center(child: CircularProgressIndicator()),
                ),
                _Stage.primaryPassword => primaryPassword(context),
                _Stage.done => summary(context),
              },
            ),
          ),
          if (error case final error?)
            Padding(
              padding: const EdgeInsets.fromLTRB(24, 0, 24, 4),
              child: Text(
                error,
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
            ),
          Padding(
            padding: const EdgeInsets.fromLTRB(24, 4, 16, 12),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: actions(context),
            ),
          ),
        ],
      ),
    ),
  );

  List<Widget> actions(BuildContext context) => switch (stage) {
    _Stage.choosing => [
      TextButton(
        onPressed: () => Navigator.pop(context),
        child: Text(found['keepsItemsOut'] != null ? 'Close' : 'Not now'),
      ),
      if (found['keepsItemsOut'] == null && profiles.isNotEmpty) ...[
        const SizedBox(width: 8),
        FilledButton(
          onPressed: chosen.isEmpty ? null : () => import(chosen.toList()),
          child: const Text('Import'),
        ),
      ],
    ],
    _Stage.finding || _Stage.importing => const [],
    _Stage.primaryPassword => [
      TextButton(
        onPressed: () {
          // This profile is left out; the next one, if any, is asked for.
          setState(() {
            waiting.removeAt(0);
            wrong = false;
            primary.clear();
            stage = waiting.isEmpty ? _Stage.done : _Stage.primaryPassword;
          });
        },
        child: const Text('Skip'),
      ),
      const SizedBox(width: 8),
      FilledButton(
        onPressed: () {
          final id = waiting.first['id'] as String;
          import([id], {id: primary.text});
        },
        child: const Text('Continue'),
      ),
    ],
    _Stage.done => [
      FilledButton(
        onPressed: () => Navigator.pop(context),
        child: const Text('Done'),
      ),
    ],
  };

  Widget choosing(BuildContext context) {
    final muted = TextStyle(color: context.colors.muted, height: 1.35);
    if (found['ok'] == false) {
      return Text(found['error'] as String? ?? 'Something went wrong.');
    }
    if (found['keepsItemsOut'] case final String org) {
      return Text(
        '$org keeps items out of Personal, so PassBundle can\'t import your browsers\' passwords there. Ask an admin for a shared folder you can add to, then export from your browser and choose Import to bring that file into the shared folder.',
        style: muted,
      );
    }
    final byBrowser = <String, List<Map<String, dynamic>>>{};
    for (final p in profiles) {
      byBrowser.putIfAbsent(browserOf(p), () => []).add(p);
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          profiles.isEmpty
              ? 'PassBundle didn\'t find a browser profile on this computer to import from.'
              : found['ask'] == true
              ? 'Choose the profiles that hold your passwords for work. PassBundle couldn\'t tell which ones are, and it reads nothing until you choose Import.'
              : 'PassBundle chose the profiles below for you. Change them if you like: it reads nothing until you choose Import.',
          style: muted,
        ),
        for (final MapEntry(key: browser, value: list) in byBrowser.entries)
          SettingsGroup(
            title: browser,
            children: [for (final p in list) profileTile(context, p)],
          ),
        for (final g in guidedBrowsers)
          SettingsGroup(
            title: browserOf(g),
            footer: g['words'] as String,
            children: [
              ListTile(
                title: const Text('Import with its own Export'),
                trailing: const Icon(Icons.chevron_right_rounded, size: 20),
                onTap: () => importWithItsExport(g['browser'] as String?),
              ),
            ],
          ),
        if (profiles.isNotEmpty)
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 16, 4, 0),
            child: Text(
              'After importing, PassBundle turns off password saving in the profiles it imported from. The passwords they saved stay where they are.',
              style: muted.copyWith(fontSize: 13),
            ),
          ),
      ],
    );
  }

  /// A profile: its name, who it's signed in as, and why it's chosen or not.
  Widget profileTile(BuildContext context, Map<String, dynamic> p) {
    final id = p['id'] as String;
    final email = p['email'] as String?;
    final notes = <String>[
      email == null ? 'Not signed in' : 'Signed in as $email',
      if (p['default'] == true) '${p['browserShort']}\'s default profile',
      if (p['managedBy'] case final String domain) 'Managed by $domain',
      switch (p['rule']) {
        'yourEmail' => 'Chosen: your PassBundle email',
        'workDomain' => 'Chosen: your organization\'s domain',
        'browserDefault' => 'Chosen: the default profile',
        _ when p['personal'] == true => 'Another account, so it\'s not chosen',
        _ => '',
      },
    ].where((n) => n.isNotEmpty).toList();
    return CheckboxListTile(
      key: ValueKey('profile $id'),
      value: chosen.contains(id),
      onChanged: (on) =>
          setState(() => on == true ? chosen.add(id) : chosen.remove(id)),
      title: Text(profileOf(p)),
      subtitle: Text(notes.join('\n')),
      isThreeLine: notes.length > 1,
      controlAffinity: ListTileControlAffinity.leading,
    );
  }

  Widget primaryPassword(BuildContext context) {
    final p = waiting.first;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          'Firefox protects the passwords in its ${profileOf(p)} profile with a Primary Password. Type it to import them. PassBundle uses it for this import only and doesn\'t keep it.',
          style: TextStyle(color: context.colors.muted, height: 1.35),
        ),
        const SizedBox(height: 16),
        TextField(
          key: const ValueKey('primary password'),
          controller: primary,
          obscureText: true,
          autofocus: true,
          enableSuggestions: false,
          autocorrect: false,
          decoration: InputDecoration(
            labelText: 'Primary Password',
            errorText: wrong
                ? 'That isn\'t this profile\'s Primary Password.'
                : null,
          ),
          onSubmitted: (_) {
            final id = p['id'] as String;
            import([id], {id: primary.text});
          },
        ),
      ],
    );
  }

  Widget summary(BuildContext context) {
    final muted = TextStyle(color: context.colors.muted, height: 1.35);
    String plural(int n, String one, String many) => n == 1 ? one : many;
    final added = this.added - toCheck;
    final lines = <String>[
      if (added > 0)
        '$added new ${plural(added, 'password is', 'passwords are')} in your vault.'
      else if (toCheck == 0 && alreadyThere > 0)
        'No new passwords: everything was already in your vault.'
      else if (toCheck == 0)
        'Nothing was imported.',
      // Besides the new ones, never among them.
      if (toCheck > 0)
        '${added > 0 ? '$toCheck more' : '$toCheck'} ${plural(toCheck, 'has', 'have')} a different password from a login you already have, so ${plural(toCheck, 'it\'s', 'they\'re')} in ${checkFolders.map((f) => '"$f"').join(' and ')} for you to compare.',
      if (alreadyThere > 0)
        '$alreadyThere ${plural(alreadyThere, 'was', 'were')} already in your vault.',
      if (empty > 0)
        '$empty saved ${plural(empty, 'sign-in had', 'sign-ins had')} no password, so ${plural(empty, 'it was', 'they were')} skipped.',
      if (unreadable > 0)
        '$unreadable couldn\'t be read. Export them from the browser to bring them in.',
    ];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final line in lines)
          Padding(
            padding: const EdgeInsets.only(bottom: 6),
            child: Text(line, style: muted),
          ),
        if (read.isNotEmpty)
          SettingsGroup(
            title: 'Imported from',
            children: [for (final r in read) turnOffTile(context, r)],
          ),
        for (final g in guided)
          SettingsGroup(
            title: profileName(g),
            footer: g['words'] as String?,
            children: [
              ListTile(
                title: const Text('Import with its own Export'),
                trailing: const Icon(Icons.chevron_right_rounded, size: 20),
                onTap: () => importWithItsExport(g['browser'] as String?),
              ),
            ],
          ),
      ],
    );
  }

  /// A profile read, and its own password saving.
  Widget turnOffTile(BuildContext context, Map<String, dynamic> r) {
    final id = r['id'] as String;
    final short = browserOf(r);
    final result = turnedOff[id];
    final count = (r['logins'] as num?)?.toInt() ?? 0;
    final saving = switch (result?['result']) {
      'done' => 'Password saving is off now.',
      'alreadyOff' => 'Password saving was already off.',
      'closeFirst' =>
        'Close $short, then choose Turn off to stop it saving passwords.',
      _ => (result?['byHand'] as String?) ?? '',
    };
    return ListTile(
      title: Text(profileName(r)),
      subtitle: Text(
        '$count saved ${count == 1 ? 'password' : 'passwords'}. $saving',
      ),
      isThreeLine: true,
      trailing: result?['result'] == 'closeFirst'
          ? TextButton(
              onPressed: () => turnOff([id]),
              child: const Text('Turn off'),
            )
          : null,
    );
  }
}
