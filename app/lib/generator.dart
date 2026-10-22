// The generator (core/src/generator.rs): a password of random characters or
// a passphrase of random words, made by the core with the choices the person
// made last time on this device. What it makes goes into the editor's field,
// so the person sees it.
import 'package:flutter/material.dart';

import 'core.dart';

/// Opens the generator; resolves to the password or passphrase to use, or
/// null if cancelled.
Future<String?> openGenerator(BuildContext context, Core core) =>
    showDialog<String>(
      context: context,
      builder: (_) => GeneratorDialog(core: core),
    );

/// The character kinds a password draws from, with what each shows.
const _kinds = {
  'uppercase': 'A-Z',
  'lowercase': 'a-z',
  'digits': '0-9',
  'symbols': '!@#\$%^&*',
};

class GeneratorDialog extends StatefulWidget {
  const GeneratorDialog({super.key, required this.core});

  final Core core;

  @override
  State<GeneratorDialog> createState() => _GeneratorDialogState();
}

class _GeneratorDialogState extends State<GeneratorDialog> {
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
    return AlertDialog(
      title: const Text('Generator'),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              SegmentedButton<String>(
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
                      child: SelectableText(
                        made ?? '',
                        key: const Key('generated'),
                        style: const TextStyle(
                          fontFamily: 'monospace',
                          fontSize: 16,
                        ),
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
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: made == null
              ? null
              : () => Navigator.of(context).pop(made),
          child: Text(passphrase ? 'Use this passphrase' : 'Use this password'),
        ),
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
      SizedBox(width: 64, child: Text(label)),
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
