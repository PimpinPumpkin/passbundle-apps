// The generator (core/src/generator.rs): a password of random characters or
// a passphrase of random words, made by the core with the choices the person
// made last time, on this device or another (core/src/synced_settings.rs).
// From the editor, what it makes goes into the field, so the person sees it;
// on its own (a phone's Generator tab, the computer's Tools), Copy copies it
// through the core, so it clears after 30 seconds like any secret. What it
// shows is in the app's own font, its digits and symbols in colors of their
// own, as the web vault shows them. What the person copied or used is its
// history, which the core keeps in memory until the vault locks.
import 'dart:async';

import 'package:flutter/material.dart';

import 'core.dart';
import 'glyphs.dart';
import 'theme.dart';
import 'widgets.dart';

/// Opens the generator; resolves to the password or passphrase to use, or
/// null if canceled. What's used goes in the generator's history.
Future<String?> openGenerator(BuildContext context, Core core) async {
  final made = await showDialog<String>(
    context: context,
    builder: (_) => GeneratorDialog(core: core),
  );
  if (made != null) core.rememberGenerated(made);
  // The choices it made with go to the person's other devices too. Not
  // waited for: offline, they go at the next sync.
  unawaited(Core.syncSettings());
  return made;
}

/// A password or passphrase as the generator shows it: in the app's own
/// font, with figures all one width, and its digits and symbols each in a
/// color of their own (sapphire and garnet, the tiles' tones), so a 0 and an
/// O, or a 1 and an l, are told apart at a glance.
class GeneratedText extends StatelessWidget {
  const GeneratedText(this.text, {super.key, this.textKey, this.size = 16});

  final String text;

  /// The key of the text itself, which tests read.
  final Key? textKey;
  final double size;

  /// Digits, and anything that's neither a letter nor a digit (a symbol,
  /// a passphrase's separator).
  static final _digit = RegExp(r'[0-9]');
  static final _letter = RegExp(r'[\p{L}\s]', unicode: true);

  @override
  Widget build(BuildContext context) {
    final brightness = Theme.of(context).brightness;
    final digits = ItemKind.login.color(brightness);
    final symbols = Categories.of('server').color(brightness);
    final spans = <TextSpan>[];
    for (final char in text.characters) {
      final color = _digit.hasMatch(char)
          ? digits
          : _letter.hasMatch(char)
          ? null
          : symbols;
      // Runs of one color, as one span each.
      if (spans.isNotEmpty && spans.last.style?.color == color) {
        final last = spans.removeLast();
        spans.add(TextSpan(text: '${last.text}$char', style: last.style));
      } else {
        spans.add(TextSpan(text: char, style: TextStyle(color: color)));
      }
    }
    return Text.rich(
      TextSpan(children: spans),
      key: textKey,
      style: TextStyle(
        fontFamily: passBundleFontFamily,
        fontSize: size,
        letterSpacing: 0.4,
        fontFeatures: const [FontFeature.tabularFigures()],
      ),
    );
  }
}

/// The generator's history: what it made that the person copied or used
/// while the vault's been open, newest first, each with Copy; and Clear.
Future<void> showGeneratorHistory(BuildContext context, Core core) =>
    showDialog<void>(
      context: context,
      builder: (_) => _GeneratorHistory(core: core),
    );

class _GeneratorHistory extends StatefulWidget {
  const _GeneratorHistory({required this.core});

  final Core core;

  @override
  State<_GeneratorHistory> createState() => _GeneratorHistoryState();
}

class _GeneratorHistoryState extends State<_GeneratorHistory> {
  late List<Map<String, dynamic>> history = widget.core.generatorHistory();

  /// What the last Copy did, said here: the dialog is over the page.
  String? said;

  Future<void> copy(String value) async {
    final answer = await widget.core.copyGenerated(value);
    if (!mounted) return;
    setState(() {
      said = answer.ok
          ? 'Copied. It clears in 30 seconds.'
          : answer.error ?? 'Couldn\'t copy.';
      history = widget.core.generatorHistory();
    });
  }

  void clear() {
    widget.core.clearGeneratorHistory();
    setState(() {
      said = null;
      history = widget.core.generatorHistory();
    });
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final scheme = Theme.of(context).colorScheme;
    return Dialog(
      clipBehavior: Clip.antiAlias,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 480, maxHeight: 640),
        child: DialogPage(
          title: 'Generator history',
          children: [
            Text(
              'What you copied or used from the generator since PassBundle '
              'was unlocked, newest first. It\'s forgotten when PassBundle '
              'locks.',
              style: TextStyle(color: c.muted, fontSize: 13, height: 1.35),
            ),
            const SizedBox(height: 14),
            if (history.isEmpty)
              Text('Nothing yet.', style: TextStyle(color: c.muted))
            else
              RestingCard(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    for (var i = 0; i < history.length; i++) ...[
                      if (i > 0) const Divider(indent: 16),
                      _HistoryRow(
                        value: history[i]['value'] as String,
                        at: (history[i]['at'] as num?)?.toInt(),
                        onCopy: () => copy(history[i]['value'] as String),
                      ),
                    ],
                  ],
                ),
              ),
            if (said case final said?)
              Padding(
                padding: const EdgeInsets.only(top: 10),
                child: Semantics(
                  liveRegion: true,
                  child: Text(said, style: TextStyle(color: c.muted)),
                ),
              ),
            if (history.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(top: 12),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: TextButton(
                    style: TextButton.styleFrom(foregroundColor: scheme.error),
                    onPressed: clear,
                    child: const Text('Clear history'),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// One password in the history: it, when, and Copy.
class _HistoryRow extends StatelessWidget {
  const _HistoryRow({required this.value, required this.at, required this.onCopy});

  final String value;
  final int? at;
  final VoidCallback onCopy;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final scheme = Theme.of(context).colorScheme;
    final when = at == null
        ? null
        : _ago(DateTime.fromMillisecondsSinceEpoch(at! * 1000));
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 8, 8),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                GeneratedText(value, size: 15),
                if (when != null)
                  Text(when, style: TextStyle(color: c.muted, fontSize: 12)),
              ],
            ),
          ),
          IconButton(
            tooltip: 'Copy',
            style: IconButton.styleFrom(
              backgroundColor: c.tint,
              foregroundColor: scheme.primary,
            ),
            icon: Glyph(Glyphs.copy, size: 20, color: scheme.primary),
            onPressed: onCopy,
          ),
        ],
      ),
    );
  }
}

/// When something happened, in the largest whole unit: "just now",
/// "5 minutes ago", "2 hours ago".
String _ago(DateTime then) {
  final minutes = DateTime.now().difference(then).inMinutes;
  if (minutes < 1) return 'Just now';
  if (minutes == 1) return '1 minute ago';
  if (minutes < 60) return '$minutes minutes ago';
  final hours = minutes ~/ 60;
  return hours == 1 ? '1 hour ago' : '$hours hours ago';
}

/// The character kinds a password draws from, with what each shows.
const _kinds = {
  'uppercase': 'A-Z',
  'lowercase': 'a-z',
  'digits': '0-9',
  'symbols': '!@#\$%^&*',
};

/// The generator in the editor: Use this password puts it in the field.
class GeneratorDialog extends StatefulWidget {
  const GeneratorDialog({super.key, required this.core});

  final Core core;

  @override
  State<GeneratorDialog> createState() => _GeneratorDialogState();
}

class _GeneratorDialogState extends State<GeneratorDialog> {
  String? made;
  bool passphrase = false;

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Generator'),
    content: SizedBox(
      width: 420,
      child: SingleChildScrollView(
        child: GeneratorPanel(
          core: widget.core,
          onMade: (made, passphrase) => setState(() {
            this.made = made;
            this.passphrase = passphrase;
          }),
        ),
      ),
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.of(context).pop(),
        child: const Text('Cancel'),
      ),
      FilledButton(
        onPressed: made == null ? null : () => Navigator.of(context).pop(made),
        child: Text(passphrase ? 'Use this passphrase' : 'Use this password'),
      ),
    ],
  );
}

/// The generator on its own: a phone's Generator tab, or the computer's
/// Tools. Copy copies what it made through the core, which clears it after
/// 30 seconds.
class GeneratorPage extends StatefulWidget {
  const GeneratorPage({super.key, required this.core, this.title = true});

  final Core core;

  /// A page's big title (a phone's tab); a dialog (a computer's Tools) has
  /// its own, is as tall as what's in it, and says what Copy did in it.
  final bool title;

  @override
  State<GeneratorPage> createState() => _GeneratorPageState();
}

class _GeneratorPageState extends State<GeneratorPage> {
  bool passphrase = false;

  /// What the panel shows: what Copy copies.
  String? made;

  @override
  void dispose() {
    // Its choices go to the person's other devices, as the editor's do.
    unawaited(Core.syncSettings());
    super.dispose();
  }

  /// In a dialog, what the last Copy did, under it.
  String? said;

  Future<void> copy() async {
    final shown = made;
    if (shown == null) return;
    final answer = await widget.core.copyGenerated(shown);
    if (!mounted) return;
    final words = answer.ok
        ? 'Copied the ${passphrase ? 'passphrase' : 'password'}. It clears in 30 seconds.'
        : answer.error ?? 'Couldn\'t copy.';
    if (!widget.title) {
      setState(() => said = words);
      return;
    }
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(words)));
  }

  @override
  Widget build(BuildContext context) {
    final children = [
      RestingCard(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
          child: GeneratorPanel(
            core: widget.core,
            onMade: (made, passphrase) => setState(() {
              this.made = made;
              this.passphrase = passphrase;
              said = null;
            }),
          ),
        ),
      ),
      const SizedBox(height: 16),
      FilledButton.icon(
        onPressed: made != null ? copy : null,
        icon: const Icon(Icons.copy_rounded, size: 18),
        label: Text(passphrase ? 'Copy passphrase' : 'Copy password'),
      ),
      if (said case final said?)
        Padding(
          padding: const EdgeInsets.only(top: 10),
          child: Semantics(
            liveRegion: true,
            child: Text(
              said,
              textAlign: TextAlign.center,
              style: TextStyle(color: context.colors.muted),
            ),
          ),
        ),
    ];
    if (!widget.title) {
      return DialogPage(
        title: 'Generator',
        padding: const EdgeInsets.fromLTRB(16, 4, 16, 24),
        children: children,
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const PageTitle('Generator'),
        Expanded(
          child: ListView(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 32),
            children: children,
          ),
        ),
      ],
    );
  }
}

/// The generator's choices and what it made, for the dialog and the page:
/// it tells [onMade] each time it makes one, and whether it's a passphrase.
class GeneratorPanel extends StatefulWidget {
  const GeneratorPanel({super.key, required this.core, required this.onMade});

  final Core core;
  final void Function(String? made, bool passphrase) onMade;

  @override
  State<GeneratorPanel> createState() => _GeneratorPanelState();
}

class _GeneratorPanelState extends State<GeneratorPanel> {
  late Map<String, dynamic> options = widget.core.generatorOptions();

  /// What the organizations require, which the choices can't go below.
  late final Map<String, dynamic> policy = widget.core.generatorPolicy();
  late final separator = TextEditingController(
    text: options['separator'] as String? ?? '-',
  );
  String? made;
  String? error;

  bool get passphrase => options['kind'] == 'passphrase';

  @override
  void initState() {
    super.initState();
    make();
  }

  /// Tells whoever shows the panel, after this frame (not while building).
  void tell() => WidgetsBinding.instance.addPostFrameCallback((_) {
    if (mounted) widget.onMade(made, passphrase);
  });

  @override
  void dispose() {
    separator.dispose();
    super.dispose();
  }

  void make() {
    final answer = widget.core.generate(options);
    made = answer.ok ? answer.body['password'] as String? : null;
    error = answer.error;
    // The choices it used, held to the policy.
    if (answer.body['options'] is Map) {
      options = Map<String, dynamic>.from(answer.body['options'] as Map);
    }
    tell();
  }

  bool required(String key) => policy[key] == true;

  /// A kind of character the policy requires, itself or by a minimum count.
  bool forced(String kind) =>
      required(kind) ||
      (kind == 'digits' && least('minDigits', 0) > 0) ||
      (kind == 'symbols' && least('minSymbols', 0) > 0);

  int least(String key, int floor) =>
      ((policy[key] as num?)?.toInt() ?? 0).clamp(floor, 1 << 20);

  bool get policySet => policy.entries.any(
    (e) => e.value != null && e.value != false && e.value != 0,
  );

  void choose(String key, Object value) => setState(() {
    options[key] = value;
    make();
  });

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final shortest = least('minLength', 4).clamp(4, 64);
    final length = (options['length'] as num? ?? 20).toInt().clamp(
      shortest,
      64,
    );
    final fewestWords = least('minWords', 3).clamp(3, 20);
    final words = (options['words'] as num? ?? 6).toInt().clamp(
      fewestWords,
      20,
    );
    final kindsOn = _kinds.keys.where((k) => options[k] == true).length;
    final onlyKind = policy['kind'] as String?;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SegmentedButton<String>(
          showSelectedIcon: false,
          segments: [
            ButtonSegment(
              value: 'password',
              label: const Text('Password'),
              enabled: onlyKind == null || onlyKind == 'password',
            ),
            ButtonSegment(
              value: 'passphrase',
              label: const Text('Passphrase'),
              enabled: onlyKind == null || onlyKind == 'passphrase',
            ),
          ],
          selected: {passphrase ? 'passphrase' : 'password'},
          onSelectionChanged: (chosen) => choose('kind', chosen.first),
        ),
        if (policySet)
          Padding(
            padding: const EdgeInsets.only(top: 12),
            child: Text(
              'Your organization sets some of these.',
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ),
        const SizedBox(height: 16),
        Container(
          padding: const EdgeInsets.fromLTRB(14, 6, 6, 6),
          decoration: BoxDecoration(
            color: scheme.surfaceContainerHighest,
            borderRadius: BorderRadius.circular(10),
          ),
          child: Row(
            children: [
              Expanded(
                // Not selectable: the system's own Copy would keep it on the
                // clipboard for good; Copy copies it, for 30 seconds.
                child: GeneratedText(
                  made ?? '',
                  textKey: const Key('generated'),
                ),
              ),
              IconButton(
                tooltip: 'Make another',
                icon: const Icon(Icons.refresh_rounded),
                onPressed: () => setState(make),
              ),
              IconButton(
                tooltip: 'Generator history',
                icon: Glyph(Glyphs.history, size: 20, color: context.colors.muted),
                onPressed: () => showGeneratorHistory(context, widget.core),
              ),
            ],
          ),
        ),
        if (error != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(error!, style: TextStyle(color: scheme.error)),
          ),
        const SizedBox(height: 8),
        if (!passphrase) ...[
          _Count(
            label: 'Length',
            value: length,
            min: shortest,
            max: 64,
            onChanged: (v) => choose('length', v),
          ),
          for (final kind in _kinds.entries)
            SwitchListTile(
              contentPadding: EdgeInsets.zero,
              title: Text(kind.value),
              value: options[kind.key] == true,
              // At least one kind stays on, and what the policy requires.
              onChanged:
                  options[kind.key] == true &&
                      (kindsOn == 1 || forced(kind.key))
                  ? null
                  : (on) => choose(kind.key, on),
            ),
          SwitchListTile(
            contentPadding: EdgeInsets.zero,
            title: const Text('Avoid look-alike characters'),
            value: options['avoidAmbiguous'] == true,
            onChanged: (on) => choose('avoidAmbiguous', on),
          ),
        ] else ...[
          _Count(
            label: 'Words',
            value: words,
            min: fewestWords,
            max: 20,
            onChanged: (v) => choose('words', v),
          ),
          TextField(
            controller: separator,
            maxLength: 3,
            decoration: const InputDecoration(
              labelText: 'Between words',
              counterText: '',
            ),
            onChanged: (v) => choose('separator', v),
          ),
          SwitchListTile(
            contentPadding: EdgeInsets.zero,
            title: const Text('Capitalize'),
            value: options['capitalize'] == true,
            onChanged: required('capitalize')
                ? null
                : (on) => choose('capitalize', on),
          ),
          SwitchListTile(
            contentPadding: EdgeInsets.zero,
            title: const Text('Include a number'),
            value: options['includeNumber'] == true,
            onChanged: required('includeNumber')
                ? null
                : (on) => choose('includeNumber', on),
          ),
        ],
      ],
    );
  }
}

/// A number chosen with a slider: a password's length, a passphrase's words.
class _Count extends StatelessWidget {
  const _Count({
    required this.label,
    required this.value,
    required this.min,
    required this.max,
    required this.onChanged,
  });

  final String label;
  final int value;
  final int min;
  final int max;
  final ValueChanged<int> onChanged;

  @override
  Widget build(BuildContext context) => Row(
    children: [
      ConstrainedBox(
        constraints: const BoxConstraints(minWidth: 64),
        child: Text(label),
      ),
      Expanded(
        // Snapping to whole numbers, without tick marks: the length's and
        // the words' sliders look alike, however many steps each has.
        child: SliderTheme(
          data: SliderTheme.of(context).copyWith(
            tickMarkShape: SliderTickMarkShape.noTickMark,
          ),
          child: Slider(
          value: value.toDouble(),
          min: min.toDouble(),
          max: max.toDouble(),
          divisions: max - min,
          label: '$value',
          semanticFormatterCallback: (v) => '${v.round()}',
          onChanged: (v) {
            if (v.round() != value) onChanged(v.round());
          },
          ),
        ),
      ),
      SizedBox(
        width: 28,
        child: Text(
          '$value',
          key: Key('$label count'),
          textAlign: TextAlign.end,
        ),
      ),
    ],
  );
}
