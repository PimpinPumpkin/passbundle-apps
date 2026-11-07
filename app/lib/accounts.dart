// Several accounts side by side (DESIGN.md in the meta repo, "Several
// accounts in the apps"): a work account and a personal one, say, each with
// its own vault, lock, and settings, up to five on a device as the browser
// extension allows. The account menu switches between them, adds one, and
// signs out of the one shown: at the foot of the menu on a computer, on the
// lock screen, and in a phone's Settings and its vault's header. Each can be
// given a name (Work, Personal) and a color from the flat tile tones, which
// label its items wherever they're listed beside other accounts': the "All
// accounts" place, and Android's autofill and passkeys. The core keeps the
// accounts apart (core/src/accounts.rs); this only draws, and asks.
import 'package:flutter/material.dart';

import 'core.dart';
import 'theme.dart';

/// The colors an account can be shown in, by the core's names for them
/// (core/src/accounts.rs, `TONES`): the flat tile tones the vault draws its
/// kinds and places in (theme.dart), so an account's label reads like a tile.
const accountTones = <String, Jewel>{
  'sapphire': ItemKind.login,
  'emerald': Jewel.allItems,
  'amethyst': ItemKind.card,
  'topaz': ItemKind.note,
  'ruby': Jewel('', Color(0xFFB01E54), Color(0xFFFF92B8)),
  'turquoise': ItemKind.identity,
  'garnet': Jewel('', Color(0xFFA8291F), Color(0xFFFF9D92)),
  'aquamarine': Jewel('', Color(0xFF1C7FB8), Color(0xFF7CC4EF)),
  'onyx': ItemKind.sshKey,
  'gold': Jewel.favorites,
};

/// Tells the screens an account's name or color changed (Settings), so the
/// menu and the labels show it.
final accountsChanged = ValueNotifier<int>(0);

/// One account signed in here, as the core lists it.
class AccountEntry {
  AccountEntry(Map<String, dynamic> json)
    : id = json['id'] as String,
      email = json['email'] as String? ?? '',
      server = json['server'] as String? ?? '',
      unlocked = json['state'] == 'unlocked',
      name = json['name'] as String?,
      tone = json['tone'] as String? ?? 'sapphire';

  /// Its id on this device.
  final String id;
  final String email;
  final String server;
  final bool unlocked;

  /// What the person calls it (Work, Personal), or null until they name it.
  final String? name;

  /// Its color, by the core's name for it ([accountTones]).
  final String tone;

  /// Its server's name.
  String get host => Uri.tryParse(server)?.host ?? server;

  /// What it's called: its name, else its email.
  String get label => name ?? email;

  /// Its color's jewel.
  Jewel get jewel => accountTones[tone] ?? ItemKind.login;
}

/// The accounts, the one the screens show, and what the account menu does
/// (main.dart does it).
class AccountsMenu {
  const AccountsMenu({
    required this.accounts,
    required this.shown,
    required this.most,
    required this.onSwitch,
    required this.onAdd,
    required this.onSignOut,
  });

  /// From what the core answers (`Core.accounts`).
  factory AccountsMenu.from(
    Map<String, dynamic> json, {
    required ValueChanged<String> onSwitch,
    required VoidCallback onAdd,
    required VoidCallback onSignOut,
  }) => AccountsMenu(
    accounts: [
      for (final a in (json['accounts'] as List? ?? const []))
        AccountEntry(Map<String, dynamic>.from(a as Map)),
    ],
    shown: json['active'] as String? ?? '',
    most: (json['most'] as num?)?.toInt() ?? 5,
    onSwitch: onSwitch,
    onAdd: onAdd,
    onSignOut: onSignOut,
  );

  final List<AccountEntry> accounts;

  /// The one the screens show, by its id.
  final String shown;

  /// One account, by its id; null for one that isn't here.
  AccountEntry? entry(String? id) {
    for (final a in accounts) {
      if (a.id == id) return a;
    }
    return null;
  }

  /// The ones unlocked now, the one shown first.
  List<AccountEntry> get unlocked => [
    for (final a in accounts)
      if (a.unlocked && a.id == shown) a,
    for (final a in accounts)
      if (a.unlocked && a.id != shown) a,
  ];

  /// How many may be signed in here.
  final int most;
  final ValueChanged<String> onSwitch;
  final VoidCallback onAdd;

  /// Signs out of the one shown, once the person says so ([askToSignOut]).
  final VoidCallback onSignOut;

  /// More than one is signed in.
  bool get several => accounts.length > 1;

  /// As many as may be: adding one waits for signing out of another.
  bool get full => accounts.length >= most;

  String get fullNote => 'Up to $most accounts on this device';
}

/// What the account menu offers: switching to an account, adding one, or
/// signing out of the one shown.
sealed class _Choice {
  const _Choice();
}

class _SwitchTo extends _Choice {
  const _SwitchTo(this.id);
  final String id;
}

class _Add extends _Choice {
  const _Add();
}

class _SignOut extends _Choice {
  const _SignOut();
}

void _chosen(AccountsMenu menu, _Choice choice) {
  switch (choice) {
    case _SwitchTo(:final id):
      if (id != menu.shown) menu.onSwitch(id);
    case _Add():
      menu.onAdd();
    case _SignOut():
      menu.onSignOut();
  }
}

/// The account menu, opened from [child]: each account (the one shown
/// marked, a locked one saying so), Add account, and Sign out of this
/// account (without it, where the screen has Sign out of its own).
class AccountMenuButton extends StatelessWidget {
  const AccountMenuButton({
    super.key,
    required this.menu,
    required this.child,
    this.signOut = true,
  });

  final AccountsMenu menu;
  final Widget child;
  final bool signOut;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return PopupMenuButton<_Choice>(
      tooltip: 'Accounts',
      position: PopupMenuPosition.over,
      onSelected: (choice) => _chosen(menu, choice),
      itemBuilder: (context) => [
        for (final a in menu.accounts)
          PopupMenuItem<_Choice>(
            key: ValueKey('account-${a.id}'),
            value: _SwitchTo(a.id),
            child: Row(
              children: [
                SizedBox(
                  width: 28,
                  child: a.id == menu.shown
                      ? const Icon(Icons.check_rounded, size: 18)
                      : null,
                ),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Row(
                        children: [
                          AccountDot(entry: a),
                          const SizedBox(width: 8),
                          Flexible(
                            child: Text(
                              a.label,
                              overflow: TextOverflow.ellipsis,
                            ),
                          ),
                        ],
                      ),
                      Text(
                        [
                          if (a.name != null) a.email else a.host,
                          if (!a.unlocked) 'Locked',
                        ].join(' · '),
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(color: c.muted, fontSize: 12),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        const PopupMenuDivider(),
        PopupMenuItem<_Choice>(
          key: const ValueKey('add-account'),
          value: const _Add(),
          enabled: !menu.full,
          child: Row(
            children: [
              const SizedBox(width: 28, child: Icon(Icons.add_rounded, size: 18)),
              Expanded(
                child: Text(
                  menu.full ? menu.fullNote : 'Add account',
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
          ),
        ),
        if (signOut)
          const PopupMenuItem<_Choice>(
            key: ValueKey('sign-out-account'),
            value: _SignOut(),
            child: Row(
              children: [
                SizedBox(
                  width: 28,
                  child: Icon(Icons.logout_rounded, size: 18),
                ),
                Expanded(child: Text('Sign out of this account')),
              ],
            ),
          ),
      ],
      child: child,
    );
  }
}

/// The account shown, at the foot of the menu on a computer: its email,
/// which opens the account menu.
class AccountFooter extends StatelessWidget {
  const AccountFooter({super.key, required this.menu, required this.email});

  final AccountsMenu menu;
  final String email;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return AccountMenuButton(
      menu: menu,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        child: Row(
          children: [
            if (menu.entry(menu.shown) case final shown? when menu.several) ...[
              AccountDot(entry: shown),
              const SizedBox(width: 6),
            ],
            Flexible(
              child: Text(
                menu.entry(menu.shown)?.label ?? email,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(color: c.muted, fontSize: 12),
              ),
            ),
            if (menu.several)
              Padding(
                padding: const EdgeInsets.only(left: 6),
                child: Text(
                  '+${menu.accounts.length - 1}',
                  style: TextStyle(color: c.muted, fontSize: 12),
                ),
              ),
            Icon(Icons.unfold_more_rounded, size: 16, color: c.muted),
          ],
        ),
      ),
    );
  }
}

/// The accounts in a phone's Settings, and on a sheet from its vault's
/// header: each account (tap to switch), Add account, and Sign out of this
/// account.
class AccountsList extends StatelessWidget {
  const AccountsList({super.key, required this.menu, this.done});

  final AccountsMenu menu;

  /// Closes the sheet it's on before acting, if it's on one.
  final VoidCallback? done;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    void choose(_Choice choice) {
      done?.call();
      _chosen(menu, choice);
    }

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final a in menu.accounts)
          ListTile(
            key: ValueKey('account-${a.id}'),
            leading: Icon(
              a.id == menu.shown
                  ? Icons.check_circle_rounded
                  : Icons.account_circle_outlined,
              color: a.id == menu.shown
                  ? Theme.of(context).colorScheme.primary
                  : c.muted,
            ),
            title: Row(
              children: [
                AccountDot(entry: a),
                const SizedBox(width: 8),
                Flexible(
                  child: Text(
                    a.label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
            subtitle: Text(
              [
                if (a.name != null) a.email else a.host,
                if (!a.unlocked) 'Locked',
              ].join(' · '),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
            onTap: () => choose(_SwitchTo(a.id)),
          ),
        ListTile(
          key: const ValueKey('add-account'),
          leading: const Icon(Icons.add_rounded),
          title: const Text('Add account'),
          subtitle: menu.full ? Text(menu.fullNote) : null,
          enabled: !menu.full,
          onTap: () => choose(const _Add()),
        ),
        ListTile(
          key: const ValueKey('sign-out-account'),
          leading: const Icon(Icons.logout_rounded),
          title: const Text('Sign out of this account'),
          onTap: () => choose(const _SignOut()),
        ),
      ],
    );
  }
}

/// The phone's account sheet, from its vault's header.
Future<void> showAccounts(BuildContext context, AccountsMenu menu) =>
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (sheet) => SafeArea(
        child: AccountsList(menu: menu, done: () => Navigator.pop(sheet)),
      ),
    );

/// Asks before signing out of an account: its vault leaves this device, and
/// so does its biometric unlock (on a computer, the browser extension's too).
/// True if the person says so. [method] is its biometric unlock's name, when
/// it's on; [email] names the account when more than one is signed in.
Future<bool> askToSignOut(
  BuildContext context, {
  String? method,
  String? email,
}) async {
  final what = email == null ? 'the vault' : 'the vault of $email';
  final sure = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text('Sign out?'),
      content: Text(
        method == null
            ? 'This removes $what from this device. Signing in again brings it back from the server.'
            : isPhone
            ? 'This removes $what from this phone and turns off unlocking with $method. Signing in again brings the vault back from the server.'
            : 'This removes $what from this device and turns off unlocking with $method, in your browser extension too. Signing in again brings the vault back from the server.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(context, true),
          child: const Text('Sign out'),
        ),
      ],
    ),
  );
  return sure == true;
}

/// An account's color, as a dot beside its name.
class AccountDot extends StatelessWidget {
  const AccountDot({super.key, required this.entry, this.size = 10});

  final AccountEntry entry;
  final double size;

  @override
  Widget build(BuildContext context) => Container(
    width: size,
    height: size,
    decoration: BoxDecoration(
      color: entry.jewel.color(Theme.of(context).brightness),
      shape: BoxShape.circle,
    ),
  );
}

/// Which account an item is in, where several accounts' are listed
/// together: its name (or email) in a pill of its color, as the item's
/// kind's tile is tinted.
class AccountBadge extends StatelessWidget {
  const AccountBadge({super.key, required this.entry});

  final AccountEntry entry;

  @override
  Widget build(BuildContext context) {
    final brightness = Theme.of(context).brightness;
    return Padding(
      padding: const EdgeInsets.only(left: 8),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 120),
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: entry.jewel.tint(brightness),
            borderRadius: BorderRadius.circular(7),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
            child: Text(
              entry.label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: entry.jewel.color(brightness),
                fontSize: 12,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Which account a new item goes in, where more than one is unlocked: each
/// unlocked one, the one shown first. Its id, or null if the person
/// cancels; the one shown's, without asking, when it's the only one.
Future<String?> chooseAccount(BuildContext context, AccountsMenu menu) async {
  final unlocked = menu.unlocked;
  if (unlocked.length < 2) return menu.shown;
  return showDialog<String>(
    context: context,
    builder: (context) => SimpleDialog(
      title: const Text('Save it in which account?'),
      children: [
        for (final a in unlocked)
          SimpleDialogOption(
            key: ValueKey('choose-account-${a.id}'),
            onPressed: () => Navigator.pop(context, a.id),
            padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
            child: Row(
              children: [
                AccountDot(entry: a, size: 12),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(a.label, overflow: TextOverflow.ellipsis),
                      if (a.name != null)
                        Text(
                          a.email,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: context.colors.muted,
                            fontSize: 12,
                          ),
                        ),
                    ],
                  ),
                ),
              ],
            ),
          ),
      ],
    ),
  );
}

/// Settings' name and color for the account shown: what it's called where
/// accounts are listed together, and the color its items are labeled in.
/// Kept on this device only.
class AccountLabelEditor extends StatefulWidget {
  const AccountLabelEditor({super.key, required this.entry});

  final AccountEntry entry;

  @override
  State<AccountLabelEditor> createState() => _AccountLabelEditorState();
}

class _AccountLabelEditorState extends State<AccountLabelEditor> {
  late final name = TextEditingController(text: widget.entry.name ?? '');
  late String tone = widget.entry.tone;
  String? error;

  @override
  void dispose() {
    name.dispose();
    super.dispose();
  }

  void save({String? tone}) {
    final answer = Core().setAccountLabel(
      name.text,
      tone,
      widget.entry.id,
    );
    setState(() {
      error = answer.ok ? null : answer.error;
      if (answer.ok && tone != null) this.tone = tone;
    });
    if (answer.ok) accountsChanged.value++;
  }

  @override
  Widget build(BuildContext context) {
    final brightness = Theme.of(context).brightness;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            key: const ValueKey('account-name'),
            controller: name,
            maxLength: 40,
            decoration: InputDecoration(
              labelText: 'Name',
              hintText: 'Work, Personal',
              helperText: widget.entry.email,
              errorText: error,
              counterText: '',
            ),
            onChanged: (_) => save(),
          ),
          const SizedBox(height: 12),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              for (final MapEntry(key: t, value: jewel) in accountTones.entries)
                Semantics(
                  label: t,
                  selected: t == tone,
                  button: true,
                  child: InkResponse(
                    key: ValueKey('account-tone-$t'),
                    onTap: () => save(tone: t),
                    radius: 18,
                    child: Container(
                      width: 28,
                      height: 28,
                      decoration: BoxDecoration(
                        color: jewel.color(brightness),
                        shape: BoxShape.circle,
                        border: t == tone
                            ? Border.all(
                                color: Theme.of(context).colorScheme.onSurface,
                                width: 2.5,
                              )
                            : null,
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ],
      ),
    );
  }
}
