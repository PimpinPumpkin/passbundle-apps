// The person's own passwords, in numbers, for their organizations (meta
// DESIGN.md, "Members' own passwords"): after each sync the core tells the
// server how many of the person's own logins are weak or reused, never which,
// only while an organization they've joined sees those numbers, which it asks
// the server first (`seen-by`). The person is told who sees them, by name and
// in the web vault's words: on Health, and on a phone in Settings too.
import 'package:flutter/material.dart';

import 'theme.dart';

/// Names in a sentence: "A", "A and B", or "A, B, and C".
String organizationNames(List<String> names) => names.length <= 2
    ? names.join(' and ')
    : '${names.sublist(0, names.length - 1).join(', ')}, and ${names.last}';

/// What the person is told, as the web vault's Health page and the browser
/// extension say it, or null while no organization sees their numbers.
String? seenByLine(List<String> names) => names.isEmpty
    ? null
    : 'Admins at ${organizationNames(names)} see how many of your own '
          'passwords are weak, reused, or exposed. Never which ones.';

/// The line, small and muted, or nothing while no organization sees the
/// numbers.
class SeenByLine extends StatelessWidget {
  const SeenByLine(this.names, {super.key});

  /// The organizations, by name, as the core has them (Core.ownHealthSeenBy).
  final List<String> names;

  @override
  Widget build(BuildContext context) {
    final line = seenByLine(names);
    if (line == null) return const SizedBox.shrink();
    return Semantics(
      container: true,
      child: Text(
        line,
        key: const ValueKey('own-health-seen-by'),
        style: TextStyle(
          color: context.colors.muted,
          fontSize: 13,
          height: 1.35,
        ),
      ),
    );
  }
}
