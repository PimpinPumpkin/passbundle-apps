// Settings' Plan group (DESIGN.md in the meta repo, "Plans and billing"): the
// person's own plan, and the plan of each organization they own or are an
// admin of, with its state and dates in words, as the server said them at the
// last sync (core/src/plans.rs). The words are the web vault's Plan page's.
//
// The apps never sell a plan: no prices, no buying, and no link to pay (the
// design's "Not yet", for Apple's and Google's rules). Nothing here works out
// a state or a date: where the server doesn't say one, it's left out. Where
// the server doesn't enforce plans, there's no Plan group at all.
import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';

/// A date the server gave (ISO 8601), in words, where the person is:
/// "December 15, 2026".
String planDate(String iso) {
  final at = DateTime.tryParse(iso)?.toLocal();
  if (at == null) return iso;
  const months = [
    'January',
    'February',
    'March',
    'April',
    'May',
    'June',
    'July',
    'August',
    'September',
    'October',
    'November',
    'December',
  ];
  return '${months[at.month - 1]} ${at.day}, ${at.year}';
}

/// A plan's name: "Premium", "Business", or "No plan" for an organization
/// with none.
String planName(Object? plan) => switch (plan) {
  'free' => 'Free',
  'premium' => 'Premium',
  'families' => 'Families',
  'business' => 'Business',
  'msp' => 'MSP seat pool',
  _ => 'No plan',
};

/// Whether the plan gives what it gives right now, as the server said.
bool planInEffect(Map<String, dynamic> plan) =>
    const {'trial', 'active', 'paymentDue'}.contains(plan['state']);

/// A plan's state and dates, a line each. The dates, and a payment due, are
/// only for whoever manages the plan: anyone else is told the state alone.
List<String> planStatus(
  Map<String, dynamic> plan, [
  String Function(String) date = planDate,
]) {
  final freeUntil = plan['freeUntil'];
  final until = plan['until'];
  final endsAt = plan['endsAt'];
  switch (plan['state']) {
    case 'trial':
      if (freeUntil is! String) return const ['Trial'];
      return [
        plan['freeWhy'] == 'offer'
            ? 'Free until ${date(freeUntil)}'
            : 'Trial until ${date(freeUntil)}',
      ];
    case 'active':
      if (until is! String) return const ['Active'];
      return [
        'Paid through ${date(until)}',
        if (plan['renews'] == true) 'Renews on ${date(until)}',
        if (plan['renews'] == false) 'Ends on ${date(until)}',
      ];
    case 'paymentDue':
      if (until is! String) return const ['Payment due'];
      return [
        endsAt is String
            ? 'Payment due. Paid through ${date(until)}; it ends on ${date(endsAt)}.'
            : 'Payment due. Paid through ${date(until)}.',
      ];
    case 'ended':
      return [endsAt is String ? 'Ended on ${date(endsAt)}' : 'Ended'];
    default:
      return const [];
  }
}

/// One row of the Plan group: whose plan, and its lines.
class PlanRow {
  const PlanRow({required this.key, required this.title, required this.lines});

  final String key;
  final String title;
  final List<String> lines;
}

/// The Plan group's rows: the person's own plan, then each organization's,
/// as the core lists them. None where the server doesn't enforce plans.
List<PlanRow> planRows(
  Map<String, dynamic> plans, [
  String Function(String) date = planDate,
]) {
  if (plans['enforced'] != true) return const [];
  final rows = <PlanRow>[];
  final own = plans['own'];
  if (own is Map) {
    final plan = Map<String, dynamic>.from(own);
    rows.add(
      PlanRow(
        key: 'plan-own',
        title: planName(plan['plan']),
        lines: [
          ...planStatus(plan, date),
          if (plans['premiumFromOrganization'] == true && !planInEffect(plan))
            "You get Premium's features through an organization or an MSP you belong to.",
          if (plan['state'] == 'ended')
            "You're on Free now. Everything you saved stays, and files you attached can still be downloaded and removed.",
        ],
      ),
    );
  }
  final organizations = plans['organizations'];
  for (final entry in organizations is List ? organizations : const []) {
    if (entry is! Map) continue;
    final org = Map<String, dynamic>.from(entry);
    final found = org['plan'];
    final plan = found is Map ? Map<String, dynamic>.from(found) : <String, dynamic>{};
    final coveredBy = plan['coveredBy'];
    final name = org['name'];
    rows.add(
      PlanRow(
        key: 'plan-org-${org['id']}',
        title: name is String ? name : '',
        lines: [
          planName(plan['plan']),
          ...planStatus(plan, date),
          if (coveredBy is Map && coveredBy['name'] is String)
            'Covered by ${coveredBy['name']}',
          if (plan['state'] == 'ended')
            "This organization's plan has ended, so nothing new can be added. Everything in it can still be opened, changed, exported, and deleted.",
        ],
      ),
    );
  }
  return rows;
}

/// Settings' Plan group, or nothing where the server doesn't enforce plans.
class PlanSettings extends StatefulWidget {
  const PlanSettings({super.key, this.plans});

  /// The plans as the core says them; the core's own when null (tests give
  /// theirs).
  final Map<String, dynamic>? plans;

  @override
  State<PlanSettings> createState() => _PlanSettingsState();
}

class _PlanSettingsState extends State<PlanSettings> {
  late final List<PlanRow> rows = planRows(widget.plans ?? Core().plans());

  @override
  Widget build(BuildContext context) {
    if (rows.isEmpty) return const SizedBox.shrink();
    return SettingsGroup(
      key: const ValueKey('plan'),
      title: 'Plan',
      children: [
        for (final row in rows)
          ListTile(
            key: ValueKey(row.key),
            title: Text(row.title),
            subtitle: row.lines.isEmpty ? null : Text(row.lines.join('\n')),
          ),
      ],
    );
  }
}
