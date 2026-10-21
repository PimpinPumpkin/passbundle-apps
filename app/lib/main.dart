// PassBundle, the prototype: unlock the sample vault from the core's test
// vectors and browse it, with the look every platform shares. Set
// PB_MEASURE=1 to unlock by itself and print timings (tools/linux/measure.sh).
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'theme.dart';

final measuring = Platform.environment['PB_MEASURE'] == '1';
const samplePassword = 'correct horse battery staple 1';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  if (measuring) {
    SchedulerBinding.instance.addPostFrameCallback((_) {
      stdout.writeln('first-frame ${DateTime.now().microsecondsSinceEpoch}');
    });
  }
  runApp(const PassBundleApp());
}

class PassBundleApp extends StatefulWidget {
  const PassBundleApp({super.key});

  @override
  State<PassBundleApp> createState() => _PassBundleAppState();
}

class _PassBundleAppState extends State<PassBundleApp> {
  ThemeMode mode = switch (Platform.environment['PB_THEME']) {
    'dark' => ThemeMode.dark,
    'light' => ThemeMode.light,
    _ => ThemeMode.system,
  };
  final core = Core();
  bool unlocked = false;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'PassBundle',
      debugShowCheckedModeBanner: false,
      theme: passBundleTheme(Brightness.light),
      darkTheme: passBundleTheme(Brightness.dark),
      themeMode: mode,
      home: unlocked
          ? VaultScreen(
              core: core,
              mode: mode,
              onMode: (m) => setState(() => mode = m),
              onLock: () {
                core.lock();
                setState(() => unlocked = false);
              },
            )
          : UnlockScreen(onUnlocked: () => setState(() => unlocked = true)),
    );
  }
}

/// The PassBundle mark: a padlock whose body is three bands.
class Mark extends StatelessWidget {
  const Mark({super.key, this.size = 28, this.color});

  final double size;
  final Color? color;

  @override
  Widget build(BuildContext context) => CustomPaint(
    size: Size(size * 26 / 32, size),
    painter: _MarkPainter(color ?? Theme.of(context).colorScheme.primary),
  );
}

class _MarkPainter extends CustomPainter {
  _MarkPainter(this.color);

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / 26, size.height / 32);
    const r85 = Radius.circular(8.5), r45 = Radius.circular(4.5);
    final path = Path()
      // The shackle.
      ..moveTo(4.5, 13)
      ..lineTo(4.5, 8.5)
      ..arcToPoint(const Offset(21.5, 8.5), radius: r85)
      ..lineTo(21.5, 13)
      ..lineTo(17.5, 13)
      ..lineTo(17.5, 8.5)
      ..arcToPoint(const Offset(8.5, 8.5), radius: r45, clockwise: false)
      ..lineTo(8.5, 13)
      ..close()
      // The top band.
      ..moveTo(4.5, 13)
      ..lineTo(21.5, 13)
      ..arcToPoint(const Offset(26, 17.5), radius: r45)
      ..lineTo(26, 18)
      ..lineTo(0, 18)
      ..lineTo(0, 17.5)
      ..arcToPoint(const Offset(4.5, 13), radius: r45)
      ..close()
      // The middle band.
      ..addRect(const Rect.fromLTWH(0, 20, 26, 5))
      // The bottom band.
      ..moveTo(0, 27)
      ..lineTo(26, 27)
      ..lineTo(26, 27.5)
      ..arcToPoint(const Offset(21.5, 32), radius: r45)
      ..lineTo(4.5, 32)
      ..arcToPoint(const Offset(0, 27.5), radius: r45)
      ..close();
    canvas.drawPath(path, Paint()..color = color);
  }

  @override
  bool shouldRepaint(_MarkPainter old) => old.color != color;
}

class UnlockScreen extends StatefulWidget {
  const UnlockScreen({super.key, required this.onUnlocked});

  final VoidCallback onUnlocked;

  @override
  State<UnlockScreen> createState() => _UnlockScreenState();
}

class _UnlockScreenState extends State<UnlockScreen> {
  final password = TextEditingController(text: measuring ? samplePassword : '');
  String? error;
  bool working = false;
  bool hidden = true;

  @override
  void initState() {
    super.initState();
    if (measuring) {
      WidgetsBinding.instance.addPostFrameCallback((_) => unlock());
    }
  }

  Future<void> unlock() async {
    setState(() {
      working = true;
      error = null;
    });
    // As the server would send them: the prelogin answer and the last sync.
    final prelogin = await rootBundle.loadString('assets/sample/prelogin.json');
    final sync = await rootBundle.loadString('assets/sample/sync.json');
    final typed = password.text;
    // The core has it now; the field doesn't keep it (docs/core.md, "Memory").
    password.clear();
    final answer = await Core.unlock(sync, prelogin, typed);
    if (!mounted) return;
    if (answer['ok'] == true) {
      if (measuring) {
        stdout.writeln('unlocked ${answer['ms']} ms, ${answer['count']} items');
      }
      widget.onUnlocked();
    } else {
      setState(() {
        working = false;
        error = answer['error'] as String?;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 380),
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const Center(child: Mark(size: 48)),
                const SizedBox(height: 20),
                Text(
                  'Unlock PassBundle',
                  textAlign: TextAlign.center,
                  style: Theme.of(context).textTheme.headlineSmall
                      ?.copyWith(fontWeight: FontWeight.w700),
                ),
                const SizedBox(height: 6),
                Text(
                  'ana.lopez@example.test',
                  textAlign: TextAlign.center,
                  style: TextStyle(color: colors.muted),
                ),
                const SizedBox(height: 28),
                TextField(
                  controller: password,
                  obscureText: hidden,
                  autofocus: true,
                  enabled: !working,
                  onSubmitted: (_) => unlock(),
                  decoration: InputDecoration(
                    labelText: 'Master password',
                    errorText: error,
                    suffixIcon: IconButton(
                      tooltip: hidden ? 'Show' : 'Hide',
                      icon: Icon(
                        hidden
                            ? Icons.visibility_outlined
                            : Icons.visibility_off_outlined,
                      ),
                      onPressed: () => setState(() => hidden = !hidden),
                    ),
                  ),
                ),
                const SizedBox(height: 14),
                FilledButton(
                  onPressed: working ? null : unlock,
                  child: working
                      ? const SizedBox(
                          width: 18,
                          height: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Text('Unlock'),
                ),
                const SizedBox(height: 18),
                Text(
                  'Prototype: the sample vault from the core\'s test vectors. Its password is "$samplePassword".',
                  textAlign: TextAlign.center,
                  style: TextStyle(color: colors.muted, fontSize: 12),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

enum Filter { all, favorites, folder, organization }

class VaultScreen extends StatefulWidget {
  const VaultScreen({
    super.key,
    required this.core,
    required this.mode,
    required this.onMode,
    required this.onLock,
  });

  final Core core;
  final ThemeMode mode;
  final ValueChanged<ThemeMode> onMode;
  final VoidCallback onLock;

  @override
  State<VaultScreen> createState() => _VaultScreenState();
}

class _VaultScreenState extends State<VaultScreen> {
  String query = '';
  Filter filter = Filter.all;
  String? filterName;
  String? selected;

  List<Map<String, dynamic>> get items {
    final all = widget.core.list(query);
    return switch (filter) {
      Filter.all => all,
      Filter.favorites => all.where((i) => i['favorite'] == true).toList(),
      Filter.folder => all.where((i) => i['folder'] == filterName).toList(),
      Filter.organization =>
        all.where((i) => i['organization'] == filterName).toList(),
    };
  }

  @override
  void initState() {
    super.initState();
    if (measuring) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        setState(
          () => selected = items.isEmpty ? null : items.first['id'] as String,
        );
        Timer(const Duration(seconds: 1), () => stdout.writeln('ready'));
      });
    }
  }

  void choose(Filter f, [String? name]) => setState(() {
    filter = f;
    filterName = name;
  });

  @override
  Widget build(BuildContext context) {
    final all = widget.core.list('');
    final folders = {
      for (final i in all)
        if (i['folder'] != null) i['folder'] as String,
    }.toList()..sort();
    final orgs = {
      for (final i in all)
        if (i['organization'] != null) i['organization'] as String,
    }.toList()..sort();
    final list = items;
    final wide = MediaQuery.sizeOf(context).width >= 900;
    final detail = selected == null
        ? null
        : ItemPane(core: widget.core, id: selected!, key: ValueKey(selected));

    final listPane = ItemList(
      header: wide
          ? null
          : PhoneHeader(
              mode: widget.mode,
              onMode: widget.onMode,
              onLock: widget.onLock,
            ),
      items: list,
      selected: selected,
      onSearch: (q) => setState(() => query = q),
      onSelect: (id) {
        if (wide) {
          setState(() => selected = id);
        } else {
          Navigator.of(context).push(
            MaterialPageRoute(
              builder: (_) => Scaffold(
                appBar: AppBar(),
                body: ItemPane(core: widget.core, id: id),
              ),
            ),
          );
        }
      },
    );

    return Scaffold(
      body: Row(
        children: [
          if (wide)
            Sidebar(
              filter: filter,
              filterName: filterName,
              folders: folders,
              organizations: orgs,
              counts: (
                all.length,
                all.where((i) => i['favorite'] == true).length,
              ),
              onChoose: choose,
              mode: widget.mode,
              onMode: widget.onMode,
              onLock: widget.onLock,
            ),
          SizedBox(
            width: wide ? 360 : MediaQuery.sizeOf(context).width,
            child: listPane,
          ),
          if (wide) const VerticalDivider(width: 1),
          if (wide)
            Expanded(
              child: Container(
                color: context.colors.pane,
                child:
                    detail ??
                    Center(
                      child: Text(
                        'Choose an item',
                        style: TextStyle(color: context.colors.muted),
                      ),
                    ),
              ),
            ),
        ],
      ),
    );
  }
}

class Sidebar extends StatelessWidget {
  const Sidebar({
    super.key,
    required this.filter,
    required this.filterName,
    required this.folders,
    required this.organizations,
    required this.counts,
    required this.onChoose,
    required this.mode,
    required this.onMode,
    required this.onLock,
  });

  final Filter filter;
  final String? filterName;
  final List<String> folders;
  final List<String> organizations;
  final (int, int) counts;
  final void Function(Filter, [String?]) onChoose;
  final ThemeMode mode;
  final ValueChanged<ThemeMode> onMode;
  final VoidCallback onLock;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget entry(
      IconData icon,
      String label,
      bool on,
      VoidCallback tap, [
      int? count,
    ]) => Padding(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 1),
      child: Material(
        color: on ? Colors.white.withValues(alpha: 0.12) : Colors.transparent,
        borderRadius: BorderRadius.circular(8),
        child: InkWell(
          borderRadius: BorderRadius.circular(8),
          onTap: tap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 9),
            child: Row(
              children: [
                Icon(icon, size: 18, color: c.sidebarText),
                const SizedBox(width: 10),
                Expanded(
                  child: Text(
                    label,
                    style: TextStyle(
                      color: c.sidebarText,
                      fontWeight: on ? FontWeight.w600 : FontWeight.w400,
                    ),
                  ),
                ),
                if (count != null)
                  Text(
                    '$count',
                    style: TextStyle(
                      color: c.sidebarText.withValues(alpha: 0.7),
                      fontSize: 12,
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
    Widget heading(String text) => Padding(
      padding: const EdgeInsets.fromLTRB(22, 18, 16, 6),
      child: Text(
        text.toUpperCase(),
        style: TextStyle(
          color: c.sidebarText.withValues(alpha: 0.6),
          fontSize: 11,
          letterSpacing: 0.8,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
    return Container(
      width: 232,
      color: c.sidebar,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 22, 16, 12),
            child: Row(
              children: [
                Mark(size: 24, color: c.sidebarText),
                const SizedBox(width: 10),
                Text(
                  'PassBundle',
                  style: TextStyle(
                    color: c.sidebarText,
                    fontWeight: FontWeight.w700,
                    fontSize: 17,
                  ),
                ),
              ],
            ),
          ),
          entry(
            Icons.grid_view_rounded,
            'All items',
            filter == Filter.all,
            () => onChoose(Filter.all),
            counts.$1,
          ),
          entry(
            Icons.star_outline_rounded,
            'Favorites',
            filter == Filter.favorites,
            () => onChoose(Filter.favorites),
            counts.$2,
          ),
          if (folders.isNotEmpty) heading('Folders'),
          for (final f in folders)
            entry(
              Icons.folder_outlined,
              f,
              filter == Filter.folder && filterName == f,
              () => onChoose(Filter.folder, f),
            ),
          if (organizations.isNotEmpty) heading('Organizations'),
          for (final o in organizations)
            entry(
              Icons.business_outlined,
              o,
              filter == Filter.organization && filterName == o,
              () => onChoose(Filter.organization, o),
            ),
          const Spacer(),
          Padding(
            padding: const EdgeInsets.all(10),
            child: Row(
              children: [
                PopupMenuButton<ThemeMode>(
                  tooltip: 'Theme',
                  icon: Icon(Icons.contrast_rounded, color: c.sidebarText),
                  initialValue: mode,
                  onSelected: onMode,
                  itemBuilder: (_) => const [
                    PopupMenuItem(
                      value: ThemeMode.system,
                      child: Text('Follow the system'),
                    ),
                    PopupMenuItem(value: ThemeMode.light, child: Text('Light')),
                    PopupMenuItem(value: ThemeMode.dark, child: Text('Dark')),
                  ],
                ),
                const Spacer(),
                TextButton.icon(
                  onPressed: onLock,
                  icon: Icon(
                    Icons.lock_outline_rounded,
                    size: 18,
                    color: c.sidebarText,
                  ),
                  label: Text('Lock', style: TextStyle(color: c.sidebarText)),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The top of the list on a phone-width window, where there's no sidebar.
class PhoneHeader extends StatelessWidget {
  const PhoneHeader({
    super.key,
    required this.mode,
    required this.onMode,
    required this.onLock,
  });

  final ThemeMode mode;
  final ValueChanged<ThemeMode> onMode;
  final VoidCallback onLock;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(20, 16, 8, 0),
    child: Row(
      children: [
        const Mark(size: 22),
        const SizedBox(width: 10),
        const Expanded(
          child: Text(
            'PassBundle',
            style: TextStyle(fontWeight: FontWeight.w700, fontSize: 17),
          ),
        ),
        PopupMenuButton<ThemeMode>(
          tooltip: 'Theme',
          icon: const Icon(Icons.contrast_rounded),
          initialValue: mode,
          onSelected: onMode,
          itemBuilder: (_) => const [
            PopupMenuItem(
              value: ThemeMode.system,
              child: Text('Follow the system'),
            ),
            PopupMenuItem(value: ThemeMode.light, child: Text('Light')),
            PopupMenuItem(value: ThemeMode.dark, child: Text('Dark')),
          ],
        ),
        IconButton(
          tooltip: 'Lock',
          icon: const Icon(Icons.lock_outline_rounded),
          onPressed: onLock,
        ),
      ],
    ),
  );
}

class ItemList extends StatelessWidget {
  const ItemList({
    super.key,
    this.header,
    required this.items,
    required this.selected,
    required this.onSearch,
    required this.onSelect,
  });

  final Widget? header;
  final List<Map<String, dynamic>> items;
  final String? selected;
  final ValueChanged<String> onSearch;
  final ValueChanged<String> onSelect;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (header != null) SafeArea(bottom: false, child: header!),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 18, 16, 12),
          child: TextField(
            onChanged: onSearch,
            decoration: const InputDecoration(
              hintText: 'Search',
              prefixIcon: Icon(Icons.search_rounded, size: 20),
            ),
          ),
        ),
        const Divider(),
        Expanded(
          child: items.isEmpty
              ? Center(
                  child: Text('Nothing here', style: TextStyle(color: c.muted)),
                )
              : ListView.builder(
                  itemCount: items.length,
                  itemBuilder: (context, i) {
                    final item = items[i];
                    final on = item['id'] == selected;
                    return Material(
                      color: on ? c.selected : Colors.transparent,
                      child: InkWell(
                        onTap: () => onSelect(item['id'] as String),
                        child: Padding(
                          padding: const EdgeInsets.symmetric(
                            horizontal: 16,
                            vertical: 11,
                          ),
                          child: Row(
                            children: [
                              Avatar(
                                name: item['name'] as String,
                                kind: item['kind'] as String,
                              ),
                              const SizedBox(width: 12),
                              Expanded(
                                child: Column(
                                  crossAxisAlignment: CrossAxisAlignment.start,
                                  children: [
                                    Text(
                                      item['name'] as String,
                                      maxLines: 1,
                                      overflow: TextOverflow.ellipsis,
                                      style: const TextStyle(
                                        fontWeight: FontWeight.w600,
                                      ),
                                    ),
                                    if ((item['subtitle'] as String).isNotEmpty)
                                      Text(
                                        item['subtitle'] as String,
                                        maxLines: 1,
                                        overflow: TextOverflow.ellipsis,
                                        style: TextStyle(
                                          color: c.muted,
                                          fontSize: 13,
                                        ),
                                      ),
                                  ],
                                ),
                              ),
                              if (item['favorite'] == true)
                                Icon(
                                  Icons.star_rounded,
                                  size: 16,
                                  color: c.muted,
                                ),
                            ],
                          ),
                        ),
                      ),
                    );
                  },
                ),
        ),
      ],
    );
  }
}

class Avatar extends StatelessWidget {
  const Avatar({
    super.key,
    required this.name,
    required this.kind,
    this.size = 36,
  });

  final String name;
  final String kind;
  final double size;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: scheme.primaryContainer,
        borderRadius: BorderRadius.circular(size * 0.28),
      ),
      alignment: Alignment.center,
      child: kind == 'note'
          ? Icon(
              Icons.sticky_note_2_outlined,
              size: size * 0.5,
              color: scheme.onPrimaryContainer,
            )
          : Text(
              name.isEmpty ? '?' : name.characters.first.toUpperCase(),
              style: TextStyle(
                color: scheme.onPrimaryContainer,
                fontWeight: FontWeight.w700,
                fontSize: size * 0.42,
              ),
            ),
    );
  }
}

class ItemPane extends StatefulWidget {
  const ItemPane({super.key, required this.core, required this.id});

  final Core core;
  final String id;

  @override
  State<ItemPane> createState() => _ItemPaneState();
}

class _ItemPaneState extends State<ItemPane> {
  final revealed = <String, String>{};
  Map<String, dynamic>? totp;
  Timer? ticker;

  @override
  void initState() {
    super.initState();
    tick();
    ticker = Timer.periodic(const Duration(seconds: 1), (_) => tick());
  }

  void tick() => setState(() => totp = widget.core.totp(widget.id));

  @override
  void dispose() {
    ticker?.cancel();
    super.dispose();
  }

  void toggle(String what) => setState(() {
    if (revealed.remove(what) == null) {
      final secret = widget.core.reveal(widget.id, what);
      if (secret != null) revealed[what] = secret;
    }
  });

  void copy(String label, String text) {
    Clipboard.setData(ClipboardData(text: text));
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text('Copied the $label'),
        duration: const Duration(seconds: 2),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final item = widget.core.item(widget.id);
    if (item == null) return const SizedBox.shrink();
    final c = context.colors;
    final rows = <Widget>[];
    Widget row(String label, Widget value, {List<Widget> actions = const []}) =>
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 12),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      label,
                      style: TextStyle(
                        color: c.muted,
                        fontSize: 12,
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                    const SizedBox(height: 3),
                    value,
                  ],
                ),
              ),
              ...actions,
            ],
          ),
        );
    Widget secretRow(String label, String what) => row(
      label,
      // Inter, like everything else: no system font, so it looks the same everywhere.
      SelectableText(
        revealed[what] ?? '••••••••••••',
        style: const TextStyle(fontSize: 15, letterSpacing: 0.4),
      ),
      actions: [
        IconButton(
          tooltip: revealed.containsKey(what) ? 'Hide' : 'Reveal',
          icon: Icon(
            revealed.containsKey(what)
                ? Icons.visibility_off_outlined
                : Icons.visibility_outlined,
            size: 20,
          ),
          onPressed: () => toggle(what),
        ),
      ],
    );

    if (item['username'] != null) {
      rows.add(
        row(
          'Username',
          SelectableText(
            item['username'] as String,
            style: const TextStyle(fontSize: 15),
          ),
          actions: [
            IconButton(
              tooltip: 'Copy',
              icon: const Icon(Icons.copy_rounded, size: 19),
              onPressed: () => copy('username', item['username'] as String),
            ),
          ],
        ),
      );
    }
    if (item['hasPassword'] == true) {
      rows.add(secretRow('Password', 'password'));
    }
    if (item['hasTotp'] == true && totp != null && totp!['code'] != null) {
      final code = totp!['code'] as String;
      final left = (totp!['secondsLeft'] as num).toInt();
      final period = (totp!['period'] as num).toInt();
      rows.add(
        row(
          'One-time code',
          Text(
            '${code.substring(0, code.length ~/ 2)} ${code.substring(code.length ~/ 2)}',
            style: const TextStyle(
              fontSize: 18,
              fontWeight: FontWeight.w600,
              letterSpacing: 1.5,
            ),
          ),
          actions: [
            SizedBox(
              width: 22,
              height: 22,
              child: CircularProgressIndicator(
                value: left / period,
                strokeWidth: 2.5,
                backgroundColor: c.line,
              ),
            ),
            const SizedBox(width: 6),
            Text('${left}s', style: TextStyle(color: c.muted, fontSize: 12)),
          ],
        ),
      );
    }
    for (final uri in (item['uris'] as List).cast<String>()) {
      rows.add(
        row(
          'Website',
          Text(
            uri,
            style: TextStyle(
              color: Theme.of(context).colorScheme.primary,
              fontSize: 15,
            ),
          ),
          actions: [
            IconButton(
              tooltip: 'Copy',
              icon: const Icon(Icons.copy_rounded, size: 19),
              onPressed: () => copy('website', uri),
            ),
          ],
        ),
      );
    }
    final fields = (item['fields'] as List).cast<Map<String, dynamic>>();
    for (var i = 0; i < fields.length; i++) {
      final f = fields[i];
      rows.add(
        f['hidden'] == true
            ? secretRow(f['name'] as String, 'field:$i')
            : row(
                f['name'] as String,
                SelectableText(
                  (f['value'] ?? '') as String,
                  style: const TextStyle(fontSize: 15),
                ),
              ),
      );
    }
    if (item['hasNotes'] == true) rows.add(secretRow('Notes', 'notes'));

    Widget card(List<Widget> children) => Container(
      decoration: BoxDecoration(
        color: Theme.of(context).colorScheme.surface,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: c.line),
      ),
      child: Column(
        children: [
          for (var i = 0; i < children.length; i++) ...[
            if (i > 0) const Divider(indent: 18, endIndent: 18),
            children[i],
          ],
        ],
      ),
    );

    return ListView(
      padding: const EdgeInsets.fromLTRB(28, 26, 28, 28),
      children: [
        Row(
          children: [
            Avatar(
              name: item['name'] as String,
              kind: item['kind'] as String,
              size: 48,
            ),
            const SizedBox(width: 16),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    item['name'] as String,
                    style: Theme.of(context).textTheme.titleLarge
                        ?.copyWith(fontWeight: FontWeight.w700),
                  ),
                  Text(
                    [
                      if (item['organization'] != null) item['organization'],
                      if (item['folder'] != null) item['folder'],
                    ].join(' · ').ifEmpty('My vault'),
                    style: TextStyle(color: c.muted, fontSize: 13),
                  ),
                ],
              ),
            ),
            if (item['favorite'] == true)
              Icon(
                Icons.star_rounded,
                color: Theme.of(context).colorScheme.primary,
              ),
          ],
        ),
        const SizedBox(height: 22),
        card(rows),
      ],
    );
  }
}

extension on String {
  String ifEmpty(String other) => isEmpty ? other : this;
}
