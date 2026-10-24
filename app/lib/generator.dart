// The generator (core/src/generator.rs): a password of random characters or
// a passphrase of random words, made by the core with the choices the person
// made last time, on this device or another (core/src/synced_settings.rs).
// From the editor, what it makes goes into the field, so the person sees it;
// on its own (a phone's Generator tab, the computer's Tools), Copy copies it
// through the core, so it clears after 30 seconds like any secret.
import 'dart:async';

import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';
import 'widgets.dart';

/// Opens the generator; resolves to the password or passphrase to use, or
/// null if cancelled.
Future<String?> openGenerator(BuildContext context, Core core) async {
  final made = await showDialog<String>(
    context: context,
    builder: (_) => GeneratorDialog(core: core),
  );
  // The choices it made with go to the person's other devices too. Not
  // waited for: offline, they go at the next sync.
  unawaited(Core.syncSettings());
  return made;
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

  /// A page's big title (a phone's tab); a dialog has its own.
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

  Future<void> copy() async {
    final shown = made;
    if (shown == null) return;
    final answer = await widget.core.copyGenerated(shown);
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(
          answer.ok
              ? 'Copied the ${passphrase ? 'passphrase' : 'password'}. It clears in 30 seconds.'
              : answer.error ?? 'Couldn\'t copy.',
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      if (widget.title) const PageTitle('Generator'),
      Expanded(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(16, 4, 16, 32),
          children: [
            RestingCard(
              child: Padding(
                padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
                child: GeneratorPanel(
                  core: widget.core,
                  onMade: (made, passphrase) => setState(() {
                    this.made = made;
                    this.passphrase = passphrase;
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
          ],
        ),
      ),
    ],
  );
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
                child: Text(
                  made ?? '',
                  key: const Key('generated'),
                  style: const TextStyle(fontFamily: 'monospace', fontSize: 16),
                ),
              ),
              IconButton(
                tooltip: 'Make another',
                icon: const Icon(Icons.refresh_rounded),
                onPressed: () => setState(make),
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
