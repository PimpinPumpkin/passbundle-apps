// The vault: a menu (all items, favorites, the categories, and the folders:
// Personal's, then each organization's shared folders), the list with
// search, and the item, side by side on a wide window, as the web vault has
// them. On a phone (or a phone-wide window), the items are a tab beside
// Settings: their kinds as colored tiles, then favorites, the folders, and
// every item, and an item opens on a page of its own. Copying goes through
// the core, so a password never passes through Dart; secrets reach the
// screen only while revealed, and never when the person's shared folder
// hides them.
import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'accounts.dart';
import 'autofill_offer.dart';
import 'core.dart';
import 'editor.dart';
import 'generator.dart';
import 'glyphs.dart';
import 'history.dart';
import 'icons.dart';
import 'new_item.dart';
import 'places.dart';
import 'settings.dart';
import 'sign_in_requests.dart';
import 'theme.dart';
import 'unlock_offer.dart';
import 'updates.dart';
import 'widgets.dart';
import 'window_chrome.dart';

/// Where a vault opens when the screens switch to its account from
/// another one's: from "All accounts" (an item picked there), choosing the
/// account for a new item, or a sign-in Android's autofill offers to save
/// (accounts.dart).
class VaultStart {
  const VaultStart({
    this.filter = Filter.all,
    this.item,
    this.newItem = false,
    this.save,
  });

  /// The place shown.
  final Filter filter;

  /// The item opened.
  final String? item;

  /// New item's picker, its account chosen already.
  final bool newItem;

  /// The sign-in to save, for the editor (with the login's `id` to update).
  final Map<String, dynamic>? save;
}

/// The menu, the list, and an item side by side, from this wide; a phone's
/// screens below it.
const wideWidth = 900.0;

/// Settings asked for from outside the vault's screens (the Mac's PassBundle
/// menu, Settings…): the vault shown opens them.
final settingsAsked = ValueNotifier<int>(0);

bool isWide(BuildContext context) =>
    MediaQuery.sizeOf(context).width >= wideWidth;

class VaultScreen extends StatefulWidget {
  const VaultScreen({
    super.key,
    required this.core,
    required this.email,
    this.server,
    this.menu,
    required this.mode,
    required this.onMode,
    required this.onLock,
    required this.onSignInAgain,
    this.syncOnOpen = true,
    this.offerBiometric = false,
    this.start,
    this.onOpenIn,
  });

  final Core core;
  final String email;
  final String? server;

  /// The accounts signed in here, and what the account menu does
  /// (accounts.dart): this vault is the one shown.
  final AccountsMenu? menu;
  final ThemeMode mode;
  final ValueChanged<ThemeMode> onMode;
  final VoidCallback onLock;

  /// Signed out on the server: sign in again to get changes.
  final VoidCallback onSignInAgain;
  final bool syncOnOpen;

  /// Just opened with the master password: a phone may offer biometric
  /// unlock for next time (unlock_offer.dart).
  final bool offerBiometric;

  /// Where it opens, when the screens switched to its account to show
  /// something (an item from "All accounts", say).
  final VaultStart? start;

  /// Shows another account's vault, opening [VaultStart] there: what's
  /// picked from another account is opened in its own account.
  final void Function(String account, VaultStart start)? onOpenIn;

  @override
  State<VaultScreen> createState() => VaultScreenState();
}

class VaultScreenState extends State<VaultScreen> {
  String query = '';
  Filter filter = Filter.all;
  String? filterId;
  String? selected;

  /// What the search finds (everything, without one).
  List<Map<String, dynamic>> all = [];

  /// Every item, whatever the search: the menu's counts.
  List<Map<String, dynamic>> everything = [];

  /// Every unlocked account's items, with several accounts signed in ("All
  /// accounts"): what the search finds there while it's shown, else all.
  List<Map<String, dynamic>> everyAccount = [];

  /// More than one account is signed in here.
  bool get several => widget.menu?.several ?? false;
  Places places = Places.empty;

  /// The logins with something to fix, by id (core/src/health.rs): judged
  /// on a background isolate after each change.
  Map<String, Map<String, dynamic>> health = const {};
  bool syncing = false;
  bool signedOut = false;

  /// This device couldn't reach the server at the last sync: how many
  /// seconds old the copy kept here is (0 when it isn't known), or null when
  /// it could. The vault says it shows that copy, instead of an error each
  /// time it opens.
  int? offline;

  /// On a phone: the items (0), the generator (1), or Settings (2).
  int tab = 0;

  /// On a phone, the search field, kept across the tabs.
  final search = TextEditingController();

  /// Coming back to the app: a sign-in autofill offered to save may be
  /// waiting. Looked for after the next frame, since coming back can lock the
  /// app first (main.dart), and then it waits for the vault after unlocking.
  late final AppLifecycleListener lifecycle = AppLifecycleListener(
    onShow: () {
      WidgetsBinding.instance.addPostFrameCallback((_) => saveOffered());
      WidgetsBinding.instance.scheduleFrame();
      // Sign-ins waiting for an answer: asked for again at once.
      signInRequests.resume();
    },
    onHide: signInRequests.pause,
  );

  /// A sign-in request open over the vault (sign_in_requests.dart): one at
  /// a time; the next waits for it.
  bool prompting = false;

  /// What's due to be said about items expiring ("2 items expire soon",
  /// core/src/expiration.rs), at the top of the vault until it's shown or
  /// dismissed. Asked for after unlocking and every six hours after.
  String? reminder;
  Timer? reminding;

  Future<void> remind() async {
    if (!mounted) return;
    final answer = await Core.reminders();
    if (!mounted || !answer.ok) return;
    if (answer.body['title'] case final String title) {
      setState(() => reminder = title);
    }
  }

  /// A sign-in waiting for this account's answer prompts over the vault:
  /// a new device's check first (it waits on its number), then a request to
  /// sign in with a device.
  Future<void> promptForSignIns() async {
    if (prompting || !mounted) return;
    final check = signInRequests.unpromptedCheck();
    final request = check == null ? signInRequests.unprompted() : null;
    if (check == null && request == null) return;
    prompting = true;
    if (check != null) {
      await reviewNewDeviceCheck(context, check, email: widget.email);
    } else if (request != null) {
      await reviewSignInRequest(context, request, email: widget.email);
    }
    prompting = false;
    if (mounted) unawaited(promptForSignIns());
  }

  @override
  void initState() {
    super.initState();
    if (widget.start case final start?) filter = start.filter;
    reload();
    lifecycle;
    updates?.addListener(updateFound);
    settingsAsked.addListener(openSettings);
    // Read now, before the first sync has the core.
    final offer = widget.offerBiometric && mayOfferBiometric(widget.core);
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      if (widget.syncOnOpen) sync();
      updateFound();
      // Expiration reminders, each said once (and on a computer, the
      // desktop agent's tray told how many).
      if (mounted) {
        unawaited(remind());
        reminding = Timer.periodic(const Duration(hours: 6), (_) => remind());
      }
      // What it was opened to show, from another account's vault.
      if (widget.start case final start?) {
        if (start.item case final id?) {
          isWide(context) ? setState(() => selected = id) : open(id);
        }
        if (start.newItem) unawaited(newItem(ask: false));
        if (start.save case final save?) {
          unawaited(edit(save['id'] as String?, save));
        }
      }
      // A sign-in to save opens the editor; the offer waits for another time.
      final saving = widget.start?.save != null || await saveOffered();
      if (!mounted) return;
      if (!saving && offer) {
        final method = await offerBiometricUnlock(context);
        if (!mounted) return;
        if (method != null) {
          showMessage(context, 'Unlocking with $method is on.');
        }
      }
      // Then, on Android, turning on autofill, until it's on or the person
      // says "Later" (autofill_offer.dart).
      if (!saving) {
        final said = await offerAutofill(context);
        if (!mounted) return;
        if (said == autofillLater) {
          // Put off: where it is from now on, a tap away.
          showMessage(
            context,
            autofillLater,
            action: ('Settings', openSettings),
          );
        } else if (said != null) {
          showMessage(context, said);
        }
      }
      // Then sign-ins waiting for this account's answer, while the vault is
      // open (not while measuring, which has no server).
      if (widget.syncOnOpen) {
        signInRequests.addListener(promptForSignIns);
        signInRequests.start();
      }
    });
  }

  /// Another account locked or unlocked: "All accounts" lists what's
  /// unlocked now, and nothing more of one that locked.
  @override
  void didUpdateWidget(VaultScreen old) {
    super.didUpdateWidget(old);
    String unlocked(AccountsMenu? menu) =>
        [for (final a in menu?.unlocked ?? const <AccountEntry>[]) a.id].join(',');
    if (unlocked(old.menu) != unlocked(widget.menu)) reload();
  }

  @override
  void dispose() {
    reminding?.cancel();
    signInRequests.removeListener(promptForSignIns);
    signInRequests.stop();
    updates?.removeListener(updateFound);
    settingsAsked.removeListener(openSettings);
    lifecycle.dispose();
    search.dispose();
    // The vault closed (it locked, or the person signed out): its websites'
    // icons go, as the core's do.
    siteIcons.forget();
    super.dispose();
  }

  /// A newer PassBundle the check on start found: a quiet notice, once (updates.dart).
  void updateFound() {
    if (mounted) showUpdateNotice(context);
  }

  /// A sign-in Android's autofill offered to save: the editor, filled in.
  /// True if there was one. A new password for a login it has comes with
  /// that login's id and account, and opens in that account; a new login,
  /// with more than one account unlocked, asks which account it goes in.
  Future<bool> saveOffered() async {
    if (!mounted) return false;
    final save = await Core.pendingSave();
    if (save == null || !mounted) return false;
    final id = save['id'] as String?;
    var account = save['account'] as String? ?? Core.account;
    if (id == null && widget.menu != null) {
      final chosen = await chooseAccount(context, widget.menu!);
      if (chosen == null || !mounted) return true;
      account = chosen;
    }
    if (account != Core.account && widget.onOpenIn != null) {
      widget.onOpenIn!(account, VaultStart(save: save));
      return true;
    }
    unawaited(edit(account == Core.account ? id : null, save));
    return true;
  }

  /// Reads the vault again from the core (never while a slow call runs).
  void reload() {
    if (busy.value) return;
    setState(() {
      final searching = filter == Filter.everyAccount ? '' : query;
      all = widget.core.list(searching);
      everything = searching.isEmpty ? all : widget.core.list('');
      everyAccount = several
          ? widget.core.listAll(filter == Filter.everyAccount ? query : '')
          : const [];
      places = Places(widget.core.places());
      if (selected != null && widget.core.item(selected!) == null) {
        selected = null;
      }
    });
    judge();
  }

  /// Health, once the vault changes: on its own isolate, as zxcvbn takes a
  /// moment for each password.
  int _judging = 0;
  Future<void> judge() async {
    final asked = ++_judging;
    final answer = await Core.health();
    if (!mounted || asked != _judging || !answer.ok) return;
    setState(() {
      health = {
        for (final i
            in ((answer.body['items'] as List?) ?? const [])
                .cast<Map<String, dynamic>>())
          i['id'] as String: i,
      };
    });
  }

  Future<void> sync() async {
    if (syncing) return;
    setState(() => syncing = true);
    final answer = await whileBusy(Core.sync);
    if (!mounted) return;
    setState(() {
      syncing = false;
      signedOut = answer.signedOut;
      offline = answer.offline
          ? (answer.body['keptSecondsAgo'] as num?)?.toInt() ?? 0
          : null;
    });
    if (!answer.ok && !answer.signedOut && !answer.offline) {
      showMessage(context, answer.error ?? 'Couldn\'t sync.');
    }
    reload();
    // iOS's suggestions above the keyboard, from what's in the vault now.
    Core.suggest();
  }

  /// Offline, said once and quietly: what the vault shows is this device's
  /// copy, from when it was last synced, and changes wait for the server.
  Widget? get offlineBanner {
    final age = offline;
    if (age == null || signedOut) return null;
    final when = age > 0
        ? ago(DateTime.now().subtract(Duration(seconds: age)).toIso8601String())
        : null;
    return MaterialBanner(
      key: const ValueKey('offline'),
      content: Text(
        'Can\'t reach your server. You\'re seeing the copy on this device'
        '${when == null ? '' : ', synced $when'}. Saving changes needs the server.',
      ),
      leading: const Icon(Icons.cloud_off_outlined),
      actions: [TextButton(onPressed: sync, child: const Text('Try again'))],
    );
  }

  List<Map<String, dynamic>> get shown => [
    for (final i in filter == Filter.everyAccount ? everyAccount : all)
      if (shows(i, filter, filterId, health: health))
        // Health says what each login has to fix.
        if (filter == Filter.health)
          {...i, 'subtitle': healthIssues(health[i['id']]!)}
        else
          i,
  ];

  /// What the list shows, by name, for its header.
  String? get placeName => switch (filter) {
    Filter.all => null,
    Filter.everyAccount => 'All accounts',
    Filter.favorites => 'Favorites',
    Filter.expiring => 'Expiring soon',
    Filter.health => 'Health',
    Filter.kind => ItemKind.of(filterId ?? '').many,
    Filter.personal => 'Personal',
    Filter.noFolder => 'No folder',
    Filter.folder => places.folder(filterId)?['label'] as String?,
    Filter.organization => places.organization(filterId)?['name'] as String?,
    Filter.collection => places.collection(filterId)?['label'] as String?,
  };

  /// Where a new item goes by default: the folder or shared folder shown,
  /// where the person may add items there (as the web vault does).
  String? get placeShown => switch (filter) {
    Filter.folder => 'personal:$filterId',
    Filter.personal || Filter.noFolder => 'personal:',
    Filter.collection when places.collection(filterId)?['canCreate'] == true =>
      () {
        final org = places.organizations
            .where(
              (o) => (o['collections'] as List).any(
                (c) => (c as Map)['id'] == filterId,
              ),
            )
            .first;
        return 'org:${org['id']}:$filterId';
      }(),
    _ => null,
  };

  /// What an empty list says, by what it shows: a search that found
  /// nothing, a clean Health, or a place with nothing in it yet.
  String get emptyNote {
    if (query.isNotEmpty) return 'Nothing matches “${query.trim()}”.';
    return switch (filter) {
      Filter.health => 'Nothing to fix: no weak, reused, or unsecured passwords.',
      Filter.favorites => 'No favorites yet. Star an item to add it here.',
      Filter.all || Filter.everyAccount => 'No items yet. New adds one.',
      Filter.folder || Filter.collection => 'Nothing in this folder yet.',
      _ => 'Nothing here yet.',
    };
  }

  void choose(Filter f, [String? id]) {
    final searchesChange = (filter == Filter.everyAccount) !=
        (f == Filter.everyAccount);
    setState(() {
      filter = f;
      filterId = id;
    });
    // The search is every account's there, and this one's elsewhere.
    if (searchesChange && query.isNotEmpty) reload();
  }

  /// An item picked from a list: opened, in its own account. One of another
  /// account's ("All accounts") shows that account's vault with it open.
  void pick(Map<String, dynamic> item) {
    final id = item['id'] as String;
    final account = item['account'] as String?;
    if (account != null && account != Core.account && widget.onOpenIn != null) {
      widget.onOpenIn!(
        account,
        VaultStart(filter: Filter.everyAccount, item: id),
      );
      return;
    }
    if (isWide(context)) {
      setState(() => selected = id);
    } else {
      open(id);
    }
  }

  /// New item: which account it goes in, with more than one unlocked (the
  /// chosen account's vault opens to make it there), then the picker (the
  /// main kinds, then templates), then its editor.
  Future<void> newItem({bool ask = true}) async {
    if (ask && widget.menu != null) {
      final account = await chooseAccount(context, widget.menu!);
      if (account == null || !mounted) return;
      if (account != Core.account && widget.onOpenIn != null) {
        widget.onOpenIn!(
          account,
          VaultStart(
            filter: filter == Filter.everyAccount ? filter : Filter.all,
            newItem: true,
          ),
        );
        return;
      }
    }
    if (!mounted) return;
    final wide = isWide(context);
    final choice = await chooseNewItem(
      context,
      templates: widget.core.templates(),
      asDialog: wide,
    );
    if (choice == null || !mounted) return;
    await edit(null, null, choice);
  }

  Future<void> edit([
    String? id,
    Map<String, dynamic>? prefill,
    NewItemChoice? choice,
  ]) async {
    final wide = isWide(context);
    // An item that asks for the master password gets it before its editor
    // opens, so the editor shows its secrets, not blanks (Android's "Save
    // to PassBundle" opens the editor straight away).
    if (id != null) {
      final item = widget.core.item(id);
      if (item != null &&
          item['reprompt'] == true &&
          item['mayUse'] != true &&
          !await askForMasterPassword(context, id, item['name'] as String)) {
        return;
      }
      if (!mounted) return;
    }
    final saved = await openEditor(
      context,
      core: widget.core,
      places: places,
      id: id,
      prefill: prefill,
      choice: choice,
      place: id == null ? placeShown : null,
      asDialog: wide,
    );
    if (saved == null || !mounted) return;
    reload();
    setState(() => selected = saved.id.isEmpty ? selected : saved.id);
    // Saved either way; without the sync after it, the rest of the vault
    // catches up next time (the item shows already).
    showMessage(
      context,
      saved.synced
          ? (id == null ? 'Saved' : 'Saved the changes')
          : '${id == null ? 'Saved' : 'Saved the changes'}. PassBundle couldn\'t sync the rest just now, and will next time.',
    );
  }

  /// The generator on its own, in a dialog (a computer's Tools).
  Future<void> openGeneratorTool() => showDialog<void>(
    context: context,
    builder: (context) => Dialog(
      clipBehavior: Clip.antiAlias,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 480, maxHeight: 720),
        child: Scaffold(
          appBar: AppBar(
            title: const Text('Generator'),
            automaticallyImplyLeading: false,
            leading: IconButton(
              tooltip: 'Close',
              icon: const Icon(Icons.close_rounded),
              onPressed: () => Navigator.of(context).pop(),
            ),
          ),
          body: GeneratorPage(core: widget.core, title: false),
        ),
      ),
    ),
  );

  /// Import, export, and the trash are the web vault's: each opens there,
  /// at [path] ("/#/tools/import").
  Future<void> openWebVault(String path) async {
    final server = widget.server;
    final opened = server != null && await Core.openWebsite('$server$path');
    if (!opened && mounted) {
      showMessage(context, 'Couldn\'t open the web vault.');
    }
  }

  /// Settings, asked for from outside (the Mac's menu) or after putting off
  /// autofill: a dialog on a computer, the tab on a phone.
  void openSettings() {
    if (!mounted) return;
    if (!isWide(context)) {
      setState(() => tab = 2);
      return;
    }
    final menu = widget.menu;
    final several = menu != null && menu.several;
    showSettingsDialog(
      context,
      account: several ? widget.email : null,
      named: menu != null && several ? menu.entry(menu.shown) : null,
      email: widget.email,
      server: widget.server,
      onSignOut: menu?.onSignOut,
    );
  }

  /// Moves an item to the trash, once the person says so: true if it went.
  Future<bool> trash(String id, String name) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Move $name to the trash?'),
        content: const Text(
          'You can restore it from the trash in the web vault.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Move to trash'),
          ),
        ],
      ),
    );
    if (sure != true || !mounted) return false;
    final answer = await whileBusy(() => Core.trash(id));
    if (!mounted) return false;
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t move it to the trash.');
      return false;
    }
    setState(() => selected = null);
    reload();
    // Restoring it is the web vault's: its trash, a tap away.
    showMessage(
      context,
      'Moved $name to the trash.',
      action: ('Open trash', () => openWebVault('/#/vault?type=trash')),
    );
    return true;
  }

  /// Makes a folder, then shows it.
  Future<void> newFolder() async {
    final name = await askFolderName(context, 'New folder');
    if (name == null || !mounted) return;
    final answer = await whileBusy(() => Core.createFolder(name));
    if (!mounted) return;
    reload();
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t make the folder.');
      return;
    }
    choose(Filter.folder, answer.body['id'] as String?);
  }

  Future<void> renameFolder(String id, String name) async {
    final renamed = await askFolderName(context, 'Rename folder', name);
    if (renamed == null || renamed == name || !mounted) return;
    final answer = await whileBusy(() => Core.renameFolder(id, renamed));
    if (!mounted) return;
    reload();
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t rename the folder.');
    }
  }

  /// Deletes a folder; its items stay, in no folder.
  Future<void> deleteFolder(String id, String name) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Delete the folder $name?'),
        content: const Text('Its items stay in Personal, in no folder.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Delete folder'),
          ),
        ],
      ),
    );
    if (sure != true || !mounted) return;
    final answer = await whileBusy(() => Core.deleteFolder(id));
    if (!mounted) return;
    reload();
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t delete the folder.');
      return;
    }
    if (filter == Filter.folder && filterId == id) choose(Filter.all);
  }

  /// On a phone, an item opens on its own page.
  void open(String id) => Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => ItemPane(
        core: widget.core,
        id: id,
        places: places,
        server: widget.server,
        phone: true,
        onEdit: () => edit(id),
        onChanged: reload,
        onTrash: (name) async {
          if (await trash(id, name) && mounted) Navigator.of(context).pop();
        },
      ),
    ),
  );

  /// What's due about items expiring, with Show (Expiring soon) and Dismiss.
  Widget? get reminderBanner => reminder == null
      ? null
      : MaterialBanner(
          key: const ValueKey('expiration-reminder'),
          content: Text(reminder!),
          leading: const Glyph(Glyphs.clock, size: 22),
          actions: [
            TextButton(
              onPressed: () => setState(() => reminder = null),
              child: const Text('Dismiss'),
            ),
            TextButton(
              onPressed: () {
                // On a phone, from any tab: the items' tab, showing them.
                setState(() {
                  reminder = null;
                  tab = 0;
                });
                choose(Filter.expiring);
              },
              child: const Text('Show'),
            ),
          ],
        );

  /// The banners under the list's search: signed out or offline, and what's
  /// expiring.
  Widget? get banners {
    final showing = [?signedOutBanner, ?offlineBanner, ?reminderBanner];
    if (showing.isEmpty) return null;
    if (showing.length == 1) return showing.single;
    return Column(mainAxisSize: MainAxisSize.min, children: showing);
  }

  Widget? get signedOutBanner => signedOut
      ? MaterialBanner(
          content: const Text(
            'You\'ve been signed out on the server. Sign in again to get changes.',
          ),
          leading: const Icon(Icons.cloud_off_outlined),
          actions: [
            TextButton(
              onPressed: widget.onSignInAgain,
              child: const Text('Sign in'),
            ),
          ],
        )
      : null;

  @override
  Widget build(BuildContext context) {
    final wide = isWide(context);
    if (!wide) return phone(context);
    final pane = selected == null
        ? null
        : ItemPane(
            key: ValueKey(selected),
            core: widget.core,
            id: selected!,
            places: places,
            server: widget.server,
            onEdit: () => edit(selected),
            onChanged: reload,
            onTrash: (name) => trash(selected!, name),
          );
    // The top band is the window's (window_chrome.dart): the menu's top, the
    // list's search, and the item's actions sit where a title bar would, so
    // the signed-out banner goes in the list, under its search.
    return Scaffold(
      body: Column(
        children: [
          Expanded(
            child: Row(
              children: [
                Sidebar(
                  email: widget.email,
                  server: widget.server,
                  menu: widget.menu,
                  everyAccount: everyAccount.length,
                  filter: filter,
                  filterId: filterId,
                  places: places,
                  items: everything,
                  toFix: health.length,
                  onGenerator: openGeneratorTool,
                  onImport: () => openWebVault('/#/tools/import'),
                  onExport: () => openWebVault('/#/tools/export'),
                  onChoose: choose,
                  onNewFolder: newFolder,
                  onRenameFolder: renameFolder,
                  onDeleteFolder: deleteFolder,
                  onMode: widget.onMode,
                  onLock: widget.onLock,
                ),
                Expanded(
                  child: ItemList(
                    title: placeName ?? 'All items',
                    banner: banners,
                    items: shown,
                    badges: !const {
                      Filter.folder,
                      Filter.noFolder,
                      Filter.collection,
                      Filter.everyAccount,
                    }.contains(filter),
                    accounts: filter == Filter.everyAccount
                        ? widget.menu
                        : null,
                    empty: emptyNote,
                    selected: selected,
                    syncing: syncing,
                    onSearch: (q) {
                      query = q;
                      reload();
                    },
                    onNew: newItem,
                    onSync: sync,
                    onSelect: pick,
                  ),
                ),
                // The item, beside the list, as the web vault's side panel:
                // up to 520 wide, the list taking the rest.
                SizedBox(
                  width: (MediaQuery.sizeOf(context).width * 0.42).clamp(
                    380,
                    552,
                  ),
                  child:
                      pane ??
                      Center(
                        child: Text(
                          'Choose an item',
                          style: TextStyle(color: context.colors.muted),
                        ),
                      ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// On a phone: the vault and Settings, as tabs, and New item over the
  /// vault. Back goes back from a search, a kind or folder, or Settings,
  /// before it leaves the app.
  Widget phone(BuildContext context) {
    final home = tab == 0 && filter == Filter.all && query.isEmpty;
    return PopScope(
      canPop: home,
      onPopInvokedWithResult: (popped, _) {
        if (popped) return;
        if (tab != 0) {
          setState(() => tab = 0);
        } else if (query.isNotEmpty) {
          clearSearch();
        } else {
          choose(Filter.all);
        }
      },
      child: Scaffold(
        body: SafeArea(
          bottom: false,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              ?signedOutBanner,
              ?offlineBanner,
              ?reminderBanner,
              Expanded(
                child: IndexedStack(
                  index: tab,
                  children: [
                    phoneVault(context),
                    // Built each time it's chosen, so it makes a new one,
                    // and shows what's so now.
                    tab == 1
                        ? GeneratorPage(core: widget.core)
                        : const SizedBox.shrink(),
                    tab == 2
                        ? SettingsPage(
                            email: widget.email,
                            server: widget.server,
                            menu: widget.menu,
                            mode: widget.mode,
                            onMode: widget.onMode,
                          )
                        : const SizedBox.shrink(),
                  ],
                ),
              ),
            ],
          ),
        ),
        floatingActionButton: tab == 0
            ? FloatingActionButton(
                tooltip: 'New item',
                onPressed: newItem,
                child: Glyph(
                  Glyphs.plus,
                  strokeWidth: 2.2,
                  color: Theme.of(context).colorScheme.onPrimary,
                ),
              )
            : null,
        bottomNavigationBar: NavigationBar(
          selectedIndex: tab,
          onDestinationSelected: (i) => setState(() => tab = i),
          // The web vault's glyphs: all items, the generator, and the gear.
          // Send, the extension's fourth, isn't in the apps yet.
          destinations: const [
            NavigationDestination(
              icon: Glyph(Glyphs.grid, size: 22),
              label: 'Items',
            ),
            NavigationDestination(
              icon: Glyph(Glyphs.generator, size: 22),
              label: 'Generator',
            ),
            NavigationDestination(
              icon: Glyph(Glyphs.settings, size: 22),
              label: 'Settings',
            ),
          ],
        ),
      ),
    );
  }

  void clearSearch() {
    search.clear();
    query = '';
    reload();
  }

  Widget phoneVault(BuildContext context) {
    final c = context.colors;
    final filtered = filter != Filter.all;
    final folder = filter == Filter.folder ? places.folder(filterId) : null;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        PageTitle(
          placeName ?? 'Items',
          leading: filtered
              ? IconButton(
                  tooltip: 'Back to all items',
                  icon: const Icon(Icons.arrow_back_rounded),
                  onPressed: () => choose(Filter.all),
                )
              : null,
          actions: [
            if (folder != null)
              FolderMenu(
                name: folder['name'] as String,
                color: c.muted,
                large: true,
                onRename: () => renameFolder(
                  folder['id'] as String,
                  folder['name'] as String,
                ),
                onDelete: () => deleteFolder(
                  folder['id'] as String,
                  folder['name'] as String,
                ),
              ),
            if (syncing)
              const SizedBox(
                width: 48,
                height: 48,
                child: Center(
                  child: SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  ),
                ),
              )
            else
              IconButton(
                tooltip: 'Sync',
                icon: const Icon(Icons.sync_rounded),
                onPressed: sync,
              ),
            // With more than one account signed in, the one shown and the
            // others, a tap away (Settings has them too).
            if (widget.menu case final menu? when menu.several)
              IconButton(
                tooltip: 'Accounts',
                icon: const Icon(Icons.account_circle_outlined),
                onPressed: () => showAccounts(context, menu),
              ),
            IconButton(
              tooltip: 'Lock',
              icon: const Icon(Icons.lock_outline_rounded),
              onPressed: widget.onLock,
            ),
          ],
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 4, 16, 8),
          child: TextField(
            controller: search,
            textInputAction: TextInputAction.search,
            onChanged: (q) {
              query = q;
              reload();
            },
            decoration: InputDecoration(
              hintText: filtered
                  ? 'Search ${placeName ?? 'items'}'
                  : 'Search items',
              prefixIcon: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 14),
                child: Glyph(Glyphs.search, size: 20, color: c.muted),
              ),
              prefixIconConstraints: const BoxConstraints(minWidth: 48),
              suffixIcon: query.isEmpty
                  ? null
                  : IconButton(
                      tooltip: 'Clear the search',
                      icon: const Icon(Icons.close_rounded),
                      onPressed: clearSearch,
                    ),
              contentPadding: const EdgeInsets.symmetric(vertical: 14),
              border: OutlineInputBorder(
                borderRadius: BorderRadius.circular(14),
                borderSide: BorderSide(color: c.line),
              ),
              enabledBorder: OutlineInputBorder(
                borderRadius: BorderRadius.circular(14),
                borderSide: BorderSide(color: c.line),
              ),
              focusedBorder: OutlineInputBorder(
                borderRadius: BorderRadius.circular(14),
                borderSide: BorderSide(
                  color: Theme.of(context).colorScheme.primary,
                  width: 1.6,
                ),
              ),
            ),
          ),
        ),
        Expanded(
          child: RefreshIndicator(
            onRefresh: sync,
            child: CustomScrollView(
              physics: const AlwaysScrollableScrollPhysics(),
              slivers: phoneSlivers(context),
            ),
          ),
        ),
      ],
    );
  }

  List<Widget> phoneSlivers(BuildContext context) {
    final c = context.colors;
    // Room under the last row for New item, which floats over it.
    const end = SliverToBoxAdapter(child: SizedBox(height: 96));
    if (filter != Filter.all || query.isNotEmpty) {
      final list = shown;
      return [
        if (list.isEmpty)
          SliverFillRemaining(
            hasScrollBody: false,
            child: Center(
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 32),
                child: Text(
                  emptyNote,
                  textAlign: TextAlign.center,
                  style: TextStyle(color: c.muted),
                ),
              ),
            ),
          )
        else ...[
          const SliverToBoxAdapter(child: SizedBox(height: 8)),
          itemCard(list),
          end,
        ],
      ];
    }
    final listed = shown;
    final favorites = listed.where((i) => i['favorite'] == true).toList();
    Widget heading(String text, [Widget? trailing]) => SliverToBoxAdapter(
      child: SectionHeading(
        text,
        padding: const EdgeInsets.fromLTRB(20, 12, 8, 2),
        height: 44,
        trailing: trailing,
      ),
    );
    int count(Filter f, [String? id]) =>
        all.where((i) => shows(i, f, id)).length;
    // Each owner on a card of its own: Personal and its folders, then each
    // organization and its shared folders.
    final owners = <List<PlaceEntry>>[];
    for (final e in PlaceEntry.all(places)) {
      if (e.owner) owners.add([]);
      owners.last.add(e);
    }
    return [
      SliverPadding(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
        sliver: SliverToBoxAdapter(
          child: CategoryTiles(
            counts: {
              for (final k in ItemKind.all) k.kind: count(Filter.kind, k.kind),
            },
            onChoose: (kind) => choose(Filter.kind, kind),
          ),
        ),
      ),
      // Every unlocked account's items together, with more than one
      // account signed in.
      if (several) ...[
        const SliverToBoxAdapter(child: SizedBox(height: 12)),
        everyAccountCard(),
      ],
      // What's expired or expires within 30 days, while there's any.
      if (count(Filter.expiring) > 0) ...[
        const SliverToBoxAdapter(child: SizedBox(height: 12)),
        expiringCard(count(Filter.expiring)),
      ],
      if (favorites.isNotEmpty) ...[heading('Favorites'), itemCard(favorites)],
      heading(
        'Folders',
        IconButton(
          tooltip: 'New folder',
          icon: const Icon(Icons.create_new_folder_outlined, size: 22),
          onPressed: newFolder,
        ),
      ),
      for (final (i, entries) in owners.indexed) ...[
        if (i > 0) const SliverToBoxAdapter(child: SizedBox(height: 10)),
        placeCard([
          for (final e in entries)
            (e, count(e.filter, e.id), () => choose(e.filter, e.id)),
        ]),
      ],
      heading('All items'),
      if (listed.isEmpty)
        SliverToBoxAdapter(
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
            child: Text(
              'Nothing here yet. New item adds one.',
              style: TextStyle(color: c.muted),
            ),
          ),
        )
      else
        itemCard(listed),
      end,
    ];
  }

  /// Rows on a resting card, built as they scroll into view.
  Widget card(int count, Widget Function(BuildContext, int, BorderRadius) row) {
    const r = Radius.circular(16);
    final c = context.colors;
    return SliverPadding(
      padding: const EdgeInsets.symmetric(horizontal: 16),
      sliver: DecoratedSliver(
        decoration: c.resting.decoration(const BorderRadius.all(r)),
        sliver: SliverList.separated(
          itemCount: count,
          itemBuilder: (context, i) => row(
            context,
            i,
            BorderRadius.vertical(
              top: i == 0 ? r : Radius.zero,
              bottom: i == count - 1 ? r : Radius.zero,
            ),
          ),
          separatorBuilder: (_, _) => const Divider(indent: 68),
        ),
      ),
    );
  }

  Widget itemCard(List<Map<String, dynamic>> items) =>
      card(items.length, (context, i, radius) {
        final item = items[i];
        final account = filter == Filter.everyAccount
            ? widget.menu?.entry(item['account'] as String?)
            : null;
        return PhoneItemRow(
          item: item,
          radius: radius,
          badge: account == null ? null : AccountBadge(entry: account),
          onTap: () => pick(item),
        );
      });

  /// "Expiring soon", on a phone: a card of its own, above the favorites.
  Widget expiringCard(int count) => card(1, (context, i, radius) {
    final c = context.colors;
    return InkWell(
      key: const ValueKey('expiring-soon'),
      borderRadius: radius,
      onTap: () => choose(Filter.expiring),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 56),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 12, 8),
          child: Row(
            children: [
              const JewelTile(jewel: Jewel.expiring, size: 40),
              const SizedBox(width: 12),
              const Expanded(
                child: Text(
                  'Expiring soon',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(fontSize: 16, fontWeight: FontWeight.w700),
                ),
              ),
              Text('$count', style: TextStyle(color: c.muted, fontSize: 14)),
              const SizedBox(width: 6),
              Glyph(Glyphs.chevron, size: 18, color: c.muted, strokeWidth: 2),
            ],
          ),
        ),
      ),
    );
  });

  /// "All accounts", on a phone: a card of its own, above the favorites.
  Widget everyAccountCard() => card(1, (context, i, radius) {
    final c = context.colors;
    return InkWell(
      key: const ValueKey('every-account'),
      borderRadius: radius,
      onTap: () => choose(Filter.everyAccount),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 56),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 12, 8),
          child: Row(
            children: [
              const JewelTile(jewel: Jewel.allItems, size: 40),
              const SizedBox(width: 12),
              const Expanded(
                child: Text(
                  'All accounts',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(fontSize: 16, fontWeight: FontWeight.w700),
                ),
              ),
              Text(
                '${everyAccount.length}',
                style: TextStyle(color: c.muted, fontSize: 14),
              ),
              const SizedBox(width: 6),
              Glyph(Glyphs.chevron, size: 18, color: c.muted, strokeWidth: 2),
            ],
          ),
        ),
      ),
    );
  });

  /// An owner and its folders, on a card: the owner first, its folders a
  /// step in, nested ones further.
  Widget placeCard(
    List<(PlaceEntry, int, VoidCallback)> places,
  ) => card(places.length, (context, i, radius) {
    final (entry, count, tap) = places[i];
    final c = context.colors;
    final primary = Theme.of(context).colorScheme.primary;
    return InkWell(
      borderRadius: radius,
      onTap: tap,
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 56),
        child: Padding(
          padding: EdgeInsets.fromLTRB(
            16 + 18.0 * math.max(0, entry.depth - 1),
            8,
            12,
            8,
          ),
          child: Row(
            children: [
              if (entry.look != null)
                JewelTile(jewel: entry.look!, size: 40)
              else
                Container(
                  width: 40,
                  height: 40,
                  decoration: BoxDecoration(
                    color: c.tint,
                    borderRadius: BorderRadius.circular(11),
                  ),
                  alignment: Alignment.center,
                  child: Glyph(entry.glyph, size: 21, color: primary),
                ),
              const SizedBox(width: 12),
              Expanded(
                child: Text(
                  entry.label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: 16,
                    fontWeight: entry.owner ? FontWeight.w700 : FontWeight.w500,
                  ),
                ),
              ),
              Text('$count', style: TextStyle(color: c.muted, fontSize: 14)),
              const SizedBox(width: 6),
              Glyph(Glyphs.chevron, size: 18, color: c.muted, strokeWidth: 2),
            ],
          ),
        ),
      ),
    );
  });
}

/// A quiet note at the bottom, with what to do next when there's an
/// [action]: its label, and what it does.
void showMessage(
  BuildContext context,
  String text, {
  (String, VoidCallback)? action,
}) => ScaffoldMessenger.of(context).showSnackBar(
  SnackBar(
    content: Text(text),
    duration: Duration(seconds: action == null ? 3 : 6),
    action: action == null
        ? null
        : SnackBarAction(label: action.$1, onPressed: action.$2),
  ),
);

/// The kinds of item on a phone, each its jewel tile with how many there
/// are: two to a row, or one when the names' words need the room (a large
/// text size, a narrow phone). Only kinds that have items, as the web vault lists
/// them.
class CategoryTiles extends StatelessWidget {
  const CategoryTiles({
    super.key,
    required this.counts,
    required this.onChoose,
  });

  final Map<String, int> counts;
  final ValueChanged<String> onChoose;

  static const _gap = 10.0;
  static const _name = TextStyle(
    fontFamily: passBundleFontFamily,
    fontSize: 15,
    fontWeight: FontWeight.w600,
  );
  static const _count = TextStyle(
    fontFamily: passBundleFontFamily,
    fontSize: 14,
  );

  double _width(String text, TextStyle style, TextScaler scaler) {
    final painter = TextPainter(
      text: TextSpan(text: text, style: style),
      textScaler: scaler,
      textDirection: TextDirection.ltr,
      maxLines: 1,
    )..layout();
    final width = painter.width;
    painter.dispose();
    return width;
  }

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final scaler = MediaQuery.textScalerOf(context);
      final kinds = ItemKind.all
          .where((k) => (counts[k.kind] ?? 0) > 0)
          .toList();
      if (kinds.isEmpty) return const SizedBox.shrink();
      // A name may take two lines ("Bank / accounts"), but no word breaks.
      final widest = kinds
          .expand((k) => k.many.split(' '))
          .map((w) => _width(w, _name, scaler))
          .reduce(math.max);
      final most = counts.values.fold(0, math.max);
      // A tile: its padding, the jewel, the name, and the count.
      final needs =
          12 + 34 + 10 + widest + 8 + _width('$most', _count, scaler) + 12;
      final columns = needs <= (constraints.maxWidth - _gap) / 2 ? 2 : 1;
      return Column(
        children: [
          for (var i = 0; i < kinds.length; i += columns) ...[
            if (i > 0) const SizedBox(height: _gap),
            IntrinsicHeight(
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (var j = i; j < i + columns; j++) ...[
                    if (j > i) const SizedBox(width: _gap),
                    Expanded(
                      child: j < kinds.length
                          ? _tile(context, kinds[j])
                          : const SizedBox.shrink(),
                    ),
                  ],
                ],
              ),
            ),
          ],
        ],
      );
    },
  );

  Widget _tile(BuildContext context, ItemKind kind) => RestingCard(
    radius: 14,
    child: InkWell(
      onTap: () => onChoose(kind.kind),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 56),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
          child: Row(
            children: [
              JewelTile(jewel: kind, size: 34),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  kind.many,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: _name,
                ),
              ),
              const SizedBox(width: 8),
              Text(
                '${counts[kind.kind] ?? 0}',
                style: _count.copyWith(color: context.colors.muted),
              ),
            ],
          ),
        ),
      ),
    ),
  );
}

/// An item's row in a phone's list: its tile, name, and what tells it apart
/// (a username, say), and that it opens.
class PhoneItemRow extends StatelessWidget {
  const PhoneItemRow({
    super.key,
    required this.item,
    required this.radius,
    required this.onTap,
    this.badge,
  });

  final Map<String, dynamic> item;
  final BorderRadius radius;
  final VoidCallback onTap;

  /// Its account's, where several accounts' items are listed together.
  final Widget? badge;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final subtitle = item['subtitle'] as String;
    return InkWell(
      borderRadius: radius,
      onTap: onTap,
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 64),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 10, 12, 10),
          child: Row(
            children: [
              Avatar.of(item, size: 40),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      item['name'] as String,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        fontSize: 16,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    if (subtitle.isNotEmpty) ...[
                      const SizedBox(height: 2),
                      Text(
                        subtitle,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(color: c.muted, fontSize: 14),
                      ),
                    ],
                  ],
                ),
              ),
              ExpiryBadge(item),
              if (item['favorite'] == true)
                const Padding(
                  padding: EdgeInsets.only(left: 6),
                  child: Glyph(
                    Glyphs.star,
                    size: 18,
                    color: favoriteEdge,
                    fill: favoriteColor,
                    strokeWidth: 1.6,
                    semanticLabel: 'Favorite',
                  ),
                ),
              ?badge,
              const SizedBox(width: 6),
              Glyph(Glyphs.chevron, size: 18, color: c.muted, strokeWidth: 2),
            ],
          ),
        ),
      ),
    );
  }
}

/// The favorite star beside an item's name: filled when it's a favorite,
/// and it toggles.
class FavoriteButton extends StatelessWidget {
  const FavoriteButton({
    super.key,
    required this.on,
    required this.onChanged,
    this.size = 20,
  });

  final bool on;
  final ValueChanged<bool> onChanged;
  final double size;

  @override
  Widget build(BuildContext context) => Semantics(
    checked: on,
    child: IconButton(
      tooltip: on ? 'Remove from favorites' : 'Add to favorites',
      onPressed: () => onChanged(!on),
      icon: Glyph(
        Glyphs.star,
        size: size,
        color: on ? favoriteEdge : context.colors.muted,
        fill: on ? favoriteColor : null,
        strokeWidth: 1.6,
      ),
    ),
  );
}

/// How long ago a date the server gave was, in the largest whole unit, as
/// the extension says it: "3 weeks ago", "yesterday", "just now".
String? ago(String? iso, [DateTime? now]) {
  final then = iso == null ? null : DateTime.tryParse(iso);
  if (then == null) return null;
  final seconds = (now ?? DateTime.now()).difference(then).inSeconds;
  if (seconds < 60) return 'just now';
  const units = [
    ('year', 365 * 24 * 3600, 'last year'),
    ('month', 30 * 24 * 3600, 'last month'),
    ('week', 7 * 24 * 3600, 'last week'),
    ('day', 24 * 3600, 'yesterday'),
    ('hour', 3600, null),
    ('minute', 60, null),
  ];
  for (final (unit, size, one) in units) {
    final n = seconds ~/ size;
    if (n >= 1) {
      if (n == 1) return one ?? '1 $unit ago';
      return '$n ${unit}s ago';
    }
  }
  return 'just now';
}

/// A favorite's star, and its edge.
const favoriteColor = Color(0xFFE8B23A);
const favoriteEdge = Color(0xFFC9921B);

/// The menu on a computer, as the web vault has it: a card on the ground
/// with All items and Favorites, the kinds of item that have items, with
/// their jewel tiles, and the folders (Personal and its folders, then each
/// organization and its shared folders, nested), each with how many it
/// holds; then who's signed in, the theme, Settings, and Lock.
class Sidebar extends StatelessWidget {
  const Sidebar({
    super.key,
    this.width = 248,
    required this.email,
    this.server,
    this.menu,
    this.everyAccount = 0,
    required this.filter,
    required this.filterId,
    required this.places,
    required this.items,
    this.toFix = 0,
    this.onGenerator,
    this.onImport,
    this.onExport,
    required this.onChoose,
    required this.onNewFolder,
    required this.onRenameFolder,
    required this.onDeleteFolder,
    required this.onMode,
    required this.onLock,
  });

  final double width;
  final String email;

  /// Its server, for Settings' account row.
  final String? server;

  /// The account menu, from the email at the foot (accounts.dart).
  final AccountsMenu? menu;

  /// How many items every unlocked account has, for "All accounts".
  final int everyAccount;
  final Filter filter;
  final String? filterId;
  final Places places;

  /// Every item, for the counts beside each entry.
  final List<Map<String, dynamic>> items;

  /// How many items Health has something to fix for.
  final int toFix;

  /// The Tools: the generator, and import and export (the web vault's,
  /// which open there).
  final VoidCallback? onGenerator;
  final VoidCallback? onImport;
  final VoidCallback? onExport;
  final void Function(Filter, [String?]) onChoose;
  final VoidCallback onNewFolder;
  final void Function(String id, String name) onRenameFolder;
  final void Function(String id, String name) onDeleteFolder;
  final ValueChanged<ThemeMode> onMode;
  final VoidCallback onLock;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    int count(Filter f, [String? id]) =>
        items.where((i) => shows(i, f, id)).length;
    // What opens in the web vault, in the browser, says so.
    final webVaultMark = Tooltip(
      message: 'Opens the web vault',
      child: Icon(Icons.open_in_new_rounded, size: 15, color: c.muted),
    );
    Widget entry({
      required Widget tile,
      required String label,
      required bool on,
      required VoidCallback tap,
      int? count,
      Widget? trailing,
      int depth = 0,
      bool strong = false,
      String? note,
    }) => Padding(
      padding: EdgeInsets.only(top: 1, bottom: 1, left: 20.0 * depth),
      child: Material(
        color: on ? c.tint : Colors.transparent,
        borderRadius: BorderRadius.circular(10),
        child: InkWell(
          borderRadius: BorderRadius.circular(10),
          onTap: tap,
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 38),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
              child: Row(
                children: [
                  SizedBox(width: 24, child: Center(child: tile)),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      label,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: 14,
                        fontWeight: on
                            ? FontWeight.w700
                            : strong
                            ? FontWeight.w600
                            : FontWeight.w500,
                      ),
                    ),
                  ),
                  if (note != null || count != null)
                    Text(
                      note ?? '$count',
                      style: TextStyle(color: c.muted, fontSize: 12),
                    ),
                  ?trailing,
                ],
              ),
            ),
          ),
        ),
      ),
    );
    Widget heading(String text, [Widget? trailing]) => Padding(
      padding: const EdgeInsets.fromLTRB(10, 14, 0, 4),
      child: Row(
        children: [
          Expanded(
            child: Semantics(
              header: true,
              child: Text(
                text.toUpperCase(),
                style: TextStyle(
                  color: c.muted,
                  fontSize: 11,
                  letterSpacing: 0.7,
                  fontWeight: FontWeight.w700,
                ),
              ),
            ),
          ),
          ?trailing,
        ],
      ),
    );
    Widget outline(String glyph, {bool strong = false}) => Glyph(
      glyph,
      size: 18,
      color: strong ? Theme.of(context).colorScheme.onSurface : c.muted,
    );
    // The kinds that have items, and the one shown.
    final kinds = ItemKind.all
        .where(
          (k) =>
              count(Filter.kind, k.kind) > 0 ||
              (filter == Filter.kind && filterId == k.kind),
        )
        .toList();
    // The window's buttons at the top left (a Mac's traffic lights, or
    // Linux's on the left) sit in the menu's top, in the window's band, and
    // the menu starts under them, as 1Password's does.
    final chrome = WindowChrome.of(context);
    final underButtons = chrome.integrated && chrome.startButtons;
    return Container(
      width: width,
      margin: const EdgeInsets.fromLTRB(12, 12, 0, 12),
      decoration: c.resting.decoration(BorderRadius.circular(16)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (underButtons) const SizedBox(height: windowBand - 12),
          // The logo, as the web vault's menu has it.
          Padding(
            padding: EdgeInsets.fromLTRB(18, underButtons ? 4 : 18, 16, 12),
            child: const Align(
              alignment: Alignment.centerLeft,
              child: Wordmark(size: 30),
            ),
          ),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.symmetric(horizontal: 8),
              children: [
                // Every unlocked account's items together, with more than
                // one account signed in; "All items" are this one's.
                if (menu?.several ?? false)
                  entry(
                    tile: const Icon(Icons.layers_outlined, size: 20),
                    label: 'All accounts',
                    on: filter == Filter.everyAccount,
                    tap: () => onChoose(Filter.everyAccount),
                    count: everyAccount,
                  ),
                entry(
                  tile: const JewelTile(jewel: Jewel.allItems, size: 24),
                  label: 'All items',
                  on: filter == Filter.all,
                  tap: () => onChoose(Filter.all),
                  count: count(Filter.all),
                ),
                entry(
                  tile: const JewelTile(jewel: Jewel.favorites, size: 24),
                  label: 'Favorites',
                  on: filter == Filter.favorites,
                  tap: () => onChoose(Filter.favorites),
                  count: count(Filter.favorites),
                ),
                // What's expired or expires within 30 days, while there's any.
                if (count(Filter.expiring) > 0 || filter == Filter.expiring)
                  entry(
                    tile: const JewelTile(jewel: Jewel.expiring, size: 24),
                    label: 'Expiring soon',
                    on: filter == Filter.expiring,
                    tap: () => onChoose(Filter.expiring),
                    count: count(Filter.expiring),
                  ),
                // Weak, reused, and unsecured passwords, and what expires,
                // as the web vault's Health.
                entry(
                  tile: const JewelTile(jewel: Jewel.health, size: 24),
                  label: 'Health',
                  on: filter == Filter.health,
                  tap: () => onChoose(Filter.health),
                  note: toFix == 0 ? null : '$toFix to fix',
                ),
                if (kinds.isNotEmpty) heading('Categories'),
                for (final k in kinds)
                  entry(
                    tile: JewelTile(jewel: k, size: 24),
                    label: k.many,
                    on: filter == Filter.kind && filterId == k.kind,
                    tap: () => onChoose(Filter.kind, k.kind),
                    count: count(Filter.kind, k.kind),
                  ),
                heading(
                  'Folders',
                  IconButton(
                    tooltip: 'New folder',
                    visualDensity: VisualDensity.compact,
                    iconSize: 18,
                    color: c.muted,
                    onPressed: onNewFolder,
                    icon: const Icon(Icons.add_rounded),
                  ),
                ),
                for (final e in PlaceEntry.all(places))
                  entry(
                    // A shared folder with its own icon and color shows
                    // them as its tile (folder_looks.dart).
                    tile: e.look != null
                        ? JewelTile(jewel: e.look!, size: 24)
                        : outline(e.glyph, strong: e.owner),
                    label: e.label,
                    depth: e.depth,
                    strong: e.owner,
                    on: filter == e.filter && filterId == e.id,
                    tap: () => onChoose(e.filter, e.id),
                    count: count(e.filter, e.id),
                    trailing: e.ownFolder
                        ? Padding(
                            padding: const EdgeInsets.only(left: 4),
                            child: FolderMenu(
                              name: places.folder(e.id)!['name'] as String,
                              color: c.muted,
                              onRename: () => onRenameFolder(
                                e.id!,
                                places.folder(e.id)!['name'] as String,
                              ),
                              onDelete: () => onDeleteFolder(
                                e.id!,
                                places.folder(e.id)!['name'] as String,
                              ),
                            ),
                          )
                        : null,
                  ),
                if (onGenerator != null || onImport != null) ...[
                  const Padding(
                    padding: EdgeInsets.symmetric(horizontal: 6, vertical: 10),
                    child: Divider(),
                  ),
                  if (onGenerator != null)
                    entry(
                      tile: outline(Glyphs.generator),
                      label: 'Generator',
                      on: false,
                      tap: onGenerator!,
                    ),
                  if (onImport != null)
                    entry(
                      tile: outline(Glyphs.download),
                      label: 'Import',
                      on: false,
                      tap: onImport!,
                      trailing: webVaultMark,
                    ),
                  if (onExport != null)
                    entry(
                      tile: outline(Glyphs.download),
                      label: 'Export',
                      on: false,
                      tap: onExport!,
                      trailing: webVaultMark,
                    ),
                ],
                const SizedBox(height: 8),
              ],
            ),
          ),
          const Divider(indent: 14, endIndent: 14),
          // The account shown: its email, which opens the account menu
          // (switching, adding one, signing out).
          if (menu case final menu?)
            Padding(
              padding: const EdgeInsets.fromLTRB(8, 4, 8, 0),
              child: AccountFooter(menu: menu, email: email),
            )
          else
            Padding(
              padding: const EdgeInsets.fromLTRB(18, 10, 16, 0),
              child: Text(
                email,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(color: c.muted, fontSize: 12),
              ),
            ),
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 4, 8, 8),
            child: Row(
              children: [
                ThemeButton(onMode: onMode, color: c.muted),
                SettingsButton(
                  color: c.muted,
                  account: (menu?.several ?? false) ? email : null,
                  named: (menu?.several ?? false)
                      ? menu!.entry(menu!.shown)
                      : null,
                  email: email,
                  server: server,
                  onSignOut: menu?.onSignOut,
                ),
                const Spacer(),
                TextButton.icon(
                  onPressed: onLock,
                  icon: const Icon(Icons.lock_outline_rounded, size: 18),
                  label: const Text('Lock'),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The list beside the menu, on a computer: search and New, what it shows
/// and how many, and the items, the chosen one raised.
class ItemList extends StatelessWidget {
  const ItemList({
    super.key,
    required this.title,
    required this.items,
    required this.selected,
    required this.syncing,
    required this.onSearch,
    required this.onNew,
    required this.onSync,
    required this.onSelect,
    this.badges = true,
    this.accounts,
    this.banner,
    this.empty = 'Nothing here yet.',
  });

  /// What it says with nothing to show: why, by what it shows.
  final String empty;

  /// What the list shows: All items, a kind, a folder.
  final String title;

  /// Said under the search: the server signed this device out.
  final Widget? banner;

  /// Where each item is, beside it (not while one folder is shown).
  final bool badges;

  /// Each item's account, beside it, where several accounts' are listed
  /// together ("All accounts").
  final AccountsMenu? accounts;
  final List<Map<String, dynamic>> items;
  final String? selected;
  final bool syncing;
  final ValueChanged<String> onSearch;
  final VoidCallback onNew;
  final VoidCallback onSync;
  final ValueChanged<Map<String, dynamic>> onSelect;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // In the window's top band (window_chrome.dart): its row, whatever
        // the platform's density, with the window's buttons in line.
        Container(
          height: windowBand - 12,
          margin: const EdgeInsets.fromLTRB(16, 12, 12, 4),
          child: Row(
            children: [
              Expanded(
                child: TextField(
                  onChanged: onSearch,
                  decoration: InputDecoration(
                    hintText: 'Search items',
                    prefixIcon: Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 12),
                      child: Glyph(Glyphs.search, size: 18, color: c.muted),
                    ),
                    prefixIconConstraints: const BoxConstraints(minWidth: 42),
                  ),
                ),
              ),
              const SizedBox(width: 4),
              syncing
                  ? const SizedBox(
                      width: 44,
                      height: 44,
                      child: Center(
                        child: SizedBox(
                          width: 18,
                          height: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        ),
                      ),
                    )
                  : IconButton(
                      tooltip: 'Sync',
                      icon: const Icon(Icons.sync_rounded),
                      onPressed: onSync,
                    ),
              const SizedBox(width: 4),
              Tooltip(
                message: 'New item',
                excludeFromSemantics: true,
                child: FilledButton.icon(
                  onPressed: onNew,
                  style: FilledButton.styleFrom(
                    minimumSize: const Size(0, 44),
                    padding: const EdgeInsets.symmetric(horizontal: 14),
                  ),
                  icon: Glyph(
                    Glyphs.plus,
                    size: 18,
                    strokeWidth: 2.4,
                    color: Theme.of(context).colorScheme.onPrimary,
                  ),
                  label: const Text('New'),
                ),
              ),
            ],
          ),
        ),
        if (banner != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 12, 0),
            child: ClipRRect(
              borderRadius: BorderRadius.circular(12),
              child: banner,
            ),
          ),
        Padding(
          padding: const EdgeInsets.fromLTRB(22, 10, 18, 6),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.baseline,
            textBaseline: TextBaseline.alphabetic,
            children: [
              Expanded(
                child: Text(
                  title,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    fontSize: 22,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ),
              Text(
                items.length == 1 ? '1 item' : '${items.length} items',
                style: TextStyle(color: c.muted, fontSize: 13),
              ),
            ],
          ),
        ),
        Expanded(
          child: items.isEmpty
              ? Center(
                  child: Padding(
                    padding: const EdgeInsets.symmetric(horizontal: 24),
                    child: Text(
                      empty,
                      textAlign: TextAlign.center,
                      style: TextStyle(color: c.muted),
                    ),
                  ),
                )
              : ListView.builder(
                  padding: const EdgeInsets.fromLTRB(10, 2, 10, 12),
                  itemCount: items.length,
                  itemBuilder: (context, i) {
                    final item = items[i];
                    final account = item['account'] as String?;
                    final on =
                        item['id'] == selected &&
                        (account == null || account == Core.account);
                    final badge = accounts?.entry(account);
                    final subtitle = item['subtitle'] as String;
                    return Padding(
                      padding: const EdgeInsets.symmetric(vertical: 2),
                      // The chosen one rises off the ground, in the
                      // selection's ring.
                      child: DecoratedBox(
                        decoration: on
                            ? c.lifted.decoration(
                                BorderRadius.circular(12),
                                ring: c.selectedRing,
                              )
                            : const BoxDecoration(),
                        child: Material(
                          color: Colors.transparent,
                          borderRadius: BorderRadius.circular(12),
                          child: InkWell(
                            borderRadius: BorderRadius.circular(12),
                            onTap: () => onSelect(item),
                            child: Padding(
                              padding: const EdgeInsets.symmetric(
                                horizontal: 12,
                                vertical: 10,
                              ),
                              child: Row(
                                children: [
                                  Avatar.of(item, size: 40),
                                  const SizedBox(width: 12),
                                  Expanded(
                                    child: Column(
                                      crossAxisAlignment:
                                          CrossAxisAlignment.start,
                                      children: [
                                        Text(
                                          item['name'] as String,
                                          maxLines: 1,
                                          overflow: TextOverflow.ellipsis,
                                          style: const TextStyle(
                                            fontSize: 15,
                                            fontWeight: FontWeight.w600,
                                          ),
                                        ),
                                        if (subtitle.isNotEmpty)
                                          Text(
                                            subtitle,
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
                                  ExpiryBadge(item),
                                  if (item['favorite'] == true)
                                    const Padding(
                                      padding: EdgeInsets.only(left: 6),
                                      child: Glyph(
                                        Glyphs.star,
                                        size: 16,
                                        color: favoriteEdge,
                                        fill: favoriteColor,
                                        strokeWidth: 1.6,
                                        semanticLabel: 'Favorite',
                                      ),
                                    ),
                                  if (badges &&
                                      (item['where'] as String?) != null)
                                    WhereBadge(item['where'] as String),
                                  if (badge != null) AccountBadge(entry: badge),
                                ],
                              ),
                            ),
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

/// Where an item is, in a quiet pill beside it in the list, as the web
/// vault's list has it: its folder, its organization's shared folder, or
/// Personal.
class WhereBadge extends StatelessWidget {
  const WhereBadge(this.where, {super.key});

  final String where;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.only(left: 8),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 110),
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: c.chip,
            borderRadius: BorderRadius.circular(7),
            border: c.chipEdge == null ? null : Border.all(color: c.chipEdge!),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
            child: Text(
              where,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: c.chipText,
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

/// An item's expiry in a pill beside it in a list, as the web vault's badge:
/// "Expires in 12 days" in amber, "Expired 5 days ago" in the error color,
/// for an item expired or expiring within 30 days (cards by their expiry
/// month); nothing otherwise.
class ExpiryBadge extends StatelessWidget {
  const ExpiryBadge(this.item, {super.key});

  final Map<String, dynamic> item;

  @override
  Widget build(BuildContext context) {
    final label = expiringLabel(item);
    if (label == null) return const SizedBox.shrink();
    final brightness = Theme.of(context).brightness;
    final tone = hasExpired(item)
        ? Theme.of(context).colorScheme.error
        : Jewel.favorites.color(brightness);
    return Padding(
      padding: const EdgeInsets.only(left: 8),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 150),
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: tone.withValues(
              alpha: brightness == Brightness.dark
                  ? Jewel.darkTint
                  : Jewel.lightTint,
            ),
            borderRadius: BorderRadius.circular(7),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
            child: Text(
              label,
              key: const ValueKey('expiry-badge'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: tone,
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

/// An item: beside the list on a computer, or on a phone a page of its own,
/// with a big tile, its name, and big copy buttons.
class ItemPane extends StatefulWidget {
  const ItemPane({
    super.key,
    required this.core,
    required this.id,
    required this.places,
    required this.onEdit,
    required this.onTrash,
    this.onChanged,
    this.server,
    this.phone = false,
  });

  /// It changed here (favorited, a version restored): the list reads again.
  final VoidCallback? onChanged;

  /// The account's server, where the web vault edits what the app doesn't.
  final String? server;

  final Core core;
  final String id;

  /// Where items live, to say where this one is.
  final Places places;

  /// Opens the editor; done when it closes.
  final Future<void> Function() onEdit;
  final ValueChanged<String> onTrash;

  /// A page of its own, with Edit and Move to trash at the top.
  final bool phone;

  @override
  State<ItemPane> createState() => _ItemPaneState();
}

class _ItemPaneState extends State<ItemPane> {
  final revealed = <String, String>{};
  Map<String, dynamic>? item;
  Map<String, dynamic>? totp;
  Timer? ticker;

  @override
  void initState() {
    super.initState();
    item = widget.core.item(widget.id);
    tick();
    ticker = Timer.periodic(const Duration(seconds: 1), (_) => tick());
  }

  void tick() {
    if (busy.value || item?['hasTotp'] != true) return;
    setState(() => totp = widget.core.totp(widget.id));
  }

  @override
  void dispose() {
    ticker?.cancel();
    masterPassword.dispose();
    super.dispose();
  }

  /// Edits it, then shows what was saved.
  Future<void> edit() async {
    await widget.onEdit();
    if (!mounted) return;
    setState(() {
      item = widget.core.item(widget.id);
      revealed.clear();
    });
  }

  void toggle(String what) => setState(() {
    if (asksAgain()) return;
    if (revealed.remove(what) == null) {
      final secret = widget.core.reveal(widget.id, what);
      if (secret != null) revealed[what] = secret;
    }
  });

  /// Whether the item asks for the master password again now (it was given
  /// for another item since): then the pane asks before anything else.
  bool asksAgain() {
    final fresh = widget.core.item(widget.id);
    if (fresh == null || fresh['reprompt'] != true || fresh['mayUse'] == true) {
      return false;
    }
    item = fresh;
    revealed.clear();
    return true;
  }

  /// Favorites it or not: the person's own mark, which even an item they
  /// can't edit takes.
  Future<void> favorite(bool on) async {
    final answer = await whileBusy(() => Core.setFavorite(widget.id, on));
    if (!mounted) return;
    if (!answer.ok) {
      showMessage(
        context,
        answer.error ??
            (on
                ? 'Couldn\'t add it to Favorites.'
                : 'Couldn\'t take it out of Favorites.'),
      );
      return;
    }
    setState(() => item = widget.core.item(widget.id));
    widget.onChanged?.call();
  }

  /// Its version history; a version restored is the item now.
  Future<void> history() async {
    if (asksAgain()) {
      setState(() {});
      return;
    }
    final restored = await openHistory(
      context,
      core: widget.core,
      id: widget.id,
      name: item?['name'] as String? ?? '',
      asDialog: !widget.phone,
    );
    if (!restored || !mounted) return;
    setState(() {
      item = widget.core.item(widget.id);
      revealed.clear();
    });
    widget.onChanged?.call();
    showMessage(context, 'Version restored');
  }

  Future<void> openSite(String uri) async {
    if (!await Core.openWebsite(uri) && mounted) {
      showMessage(context, 'Couldn\'t open $uri.');
    }
  }

  /// The web vault, with this item's editor open: it edits SSH keys and the
  /// newer kinds, which the app doesn't yet.
  Future<void> editInWebVault() => openSite(
    '${widget.server}/#/vault?itemId=${Uri.encodeQueryComponent(widget.id)}&action=edit',
  );

  /// Whether the person may do something with the item (`edit`,
  /// `seePasswords`, `delete`, ...), as their shared folders allow (the
  /// core says yes for their own items); no when the core doesn't say.
  bool may(Map<String, dynamic> item, String what) =>
      (item['permissions'] as Map?)?[what] == true;

  /// Downloads, opens, and saves an attachment to Downloads, through the core.
  Future<void> saveAttachment(String attachmentId, String name) async {
    if (asksAgain()) {
      setState(() {});
      return;
    }
    final answer = await whileBusy(
      () => Core.saveAttachment(widget.id, attachmentId, name),
    );
    if (!mounted) return;
    showMessage(
      context,
      answer.ok
          ? 'Saved to ${answer.body['path']}'
          : answer.error ?? 'Couldn\'t save that.',
    );
  }

  /// Through the core, so the value never passes through Dart; secrets clear after 30 seconds.
  Future<void> copy(String label, String what, {bool secret = true}) async {
    if (asksAgain()) {
      setState(() {});
      return;
    }
    final answer = await widget.core.copy(widget.id, what);
    if (!mounted) return;
    showMessage(
      context,
      answer.ok
          ? (secret
                ? 'Copied the $label. It clears in 30 seconds.'
                : 'Copied the $label')
          : answer.error ?? 'Couldn\'t copy.',
    );
  }

  /// The master password, for an item that asks for it again.
  final masterPassword = TextEditingController();
  String? promptError;
  bool confirming = false;

  Future<void> confirm() async {
    if (masterPassword.text.isEmpty) return;
    setState(() {
      confirming = true;
      promptError = null;
    });
    final answer = await whileBusy(
      () => Core.confirmMasterPassword(widget.id, masterPassword.text),
    );
    if (!mounted) return;
    masterPassword.clear();
    setState(() {
      confirming = false;
      if (answer.ok) {
        item = widget.core.item(widget.id);
      } else {
        promptError = answer.error;
      }
    });
    tick();
  }

  /// What an item that asks for the master password shows until it's given:
  /// its name, and the question.
  Widget askForMasterPassword(BuildContext context, Map<String, dynamic> item) {
    final c = context.colors;
    return ListView(
      padding: widget.phone
          ? const EdgeInsets.fromLTRB(20, 8, 20, 28)
          : const EdgeInsets.fromLTRB(28, 14, 28, 28),
      children: [
        Row(
          children: [
            Avatar.of(item, size: 48),
            const SizedBox(width: 16),
            Expanded(
              child: Text(
                item['name'] as String,
                style: Theme.of(context).textTheme.titleLarge
                    ?.copyWith(fontWeight: FontWeight.w700),
              ),
            ),
          ],
        ),
        const SizedBox(height: 22),
        Text(
          'This item asks for your master password.',
          style: TextStyle(color: c.muted),
        ),
        const SizedBox(height: 12),
        TextField(
          controller: masterPassword,
          obscureText: true,
          autofocus: true,
          decoration: InputDecoration(
            labelText: 'Master password',
            errorText: promptError,
          ),
          onSubmitted: (_) => confirm(),
        ),
        const SizedBox(height: 12),
        Align(
          alignment: Alignment.centerLeft,
          child: WorkingButton(
            label: 'Continue',
            working: confirming,
            onPressed: confirm,
          ),
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final item = this.item;
    if (item == null) {
      return widget.phone
          ? Scaffold(appBar: AppBar())
          : const SizedBox.shrink();
    }
    final asking = item['reprompt'] == true && item['mayUse'] != true;
    final body = asking
        ? askForMasterPassword(context, item)
        : details(context, item);
    if (!widget.phone) {
      // On a computer, its actions along the window's top band, where a
      // title bar would be, as 1Password has them, short of the window's
      // own buttons (window_chrome.dart); the item under them.
      final end = WindowChrome.of(context).endInset;
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: EdgeInsets.fromLTRB(12, 12, math.max(20, end + 8), 0),
            child: SizedBox(
              height: windowBand - 12,
              child: Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: asking ? const [] : actions(item),
              ),
            ),
          ),
          Expanded(child: body),
        ],
      );
    }
    return Scaffold(
      appBar: AppBar(
        actions: [if (!asking) ...actions(item), const SizedBox(width: 8)],
      ),
      body: body,
    );
  }

  /// What can be done with it: favorite it, edit it, and move it to the trash.
  List<Widget> actions(Map<String, dynamic> item) => [
    FavoriteButton(
      on: item['favorite'] == true,
      size: widget.phone ? 22 : 20,
      onChanged: favorite,
    ),
    // Some kinds are the web vault's to edit, for now (SSH keys and the
    // newer kinds), which it says, with the way there; and some items the
    // person's shared folder lets them see only.
    if (item['editable'] != false)
      Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4),
        child: OutlinedButton(onPressed: edit, child: const Text('Edit')),
      )
    else if (item['editInWebVault'] == true && widget.server != null)
      // A phone's top bar has room for an icon only.
      widget.phone
          ? IconButton(
              key: const ValueKey('edit-in-web-vault'),
              tooltip: 'Edit in web vault',
              icon: const Icon(Icons.open_in_new_rounded),
              onPressed: editInWebVault,
            )
          : Padding(
              padding: const EdgeInsets.symmetric(horizontal: 4),
              child: Tooltip(
                message: 'PassBundle edits this kind in the web vault, for now',
                child: OutlinedButton.icon(
                  key: const ValueKey('edit-in-web-vault'),
                  onPressed: editInWebVault,
                  icon: const Icon(Icons.open_in_new_rounded, size: 16),
                  label: const Text('Edit in web vault'),
                ),
              ),
            ),
    if (may(item, 'delete'))
      IconButton(
        tooltip: 'Move to trash',
        icon: const Icon(Icons.delete_outline_rounded),
        onPressed: () => widget.onTrash(item['name'] as String),
      ),
  ];

  /// Its details, each a row on a card with what can be done with it.
  Widget details(BuildContext context, Map<String, dynamic> item) {
    final c = context.colors;
    final scheme = Theme.of(context).colorScheme;
    final phone = widget.phone;
    final valueSize = phone ? 17.0 : 16.0;
    final rows = <Widget>[];
    Widget row(String label, Widget value, {List<Widget> actions = const []}) =>
        ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 68),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 8, 8),
            child: Row(
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(
                        label,
                        style: TextStyle(
                          color: c.muted,
                          fontSize: 13,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      const SizedBox(height: 3),
                      value,
                    ],
                  ),
                ),
                for (final a in actions) ...[const SizedBox(width: 4), a],
              ],
            ),
          ),
        );
    // Big, tinted, and in the brand color: the thing people come here to do.
    Widget action(String tooltip, Widget icon, VoidCallback onPressed) =>
        IconButton(
          tooltip: tooltip,
          style: IconButton.styleFrom(
            backgroundColor: c.tint,
            foregroundColor: scheme.primary,
            minimumSize: const Size(44, 44),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(12),
            ),
          ),
          icon: icon,
          onPressed: onPressed,
        );
    final copyGlyph = Glyph(Glyphs.copy, size: 20, color: scheme.primary);
    Widget copyButton(String label, String what, {bool secret = true}) =>
        action(
          'Copy ${label.toLowerCase()}',
          copyGlyph,
          () => copy(label, what, secret: secret),
        );
    // Whether the person's shared folder lets them see the item's secrets.
    final sees = may(item, 'seePasswords');
    Widget secretRow(String label, String what, {bool canCopy = true}) => row(
      label,
      // Google Sans Flex, like everything else: no system font, so it looks
      // the same everywhere. Not selectable: the system's own Copy would put
      // a secret on the clipboard for good, past the 30 seconds; its Copy
      // button copies it.
      Text(
        revealed[what] ?? '••••••••••••',
        style: TextStyle(fontSize: valueSize, letterSpacing: 0.6),
      ),
      actions: [
        if (sees || what == 'notes')
          IconButton(
            tooltip: revealed.containsKey(what) ? 'Hide' : 'Reveal',
            icon: Glyph(
              revealed.containsKey(what) ? Glyphs.eyeOff : Glyphs.eye,
              size: 20,
              color: c.muted,
            ),
            onPressed: () => toggle(what),
          ),
        if (canCopy && sees) copyButton(label.toLowerCase(), what),
      ],
    );

    if (item['username'] != null) {
      rows.add(
        row(
          'Username',
          SelectableText(
            item['username'] as String,
            style: TextStyle(fontSize: valueSize),
          ),
          actions: [copyButton('username', 'username', secret: false)],
        ),
      );
    }
    if (item['hasPassword'] == true) {
      rows.add(secretRow('Password', 'password'));
    }
    final code = totp?['code'] as String?;
    if (item['hasTotp'] == true && !sees) {
      rows.add(secretRow('One-time code', 'totp'));
    } else if (item['hasTotp'] == true && code != null) {
      final left = (totp!['secondsLeft'] as num).toInt();
      final period = (totp!['period'] as num).toInt();
      rows.add(
        row(
          'One-time code',
          Text(
            '${code.substring(0, code.length ~/ 2)} ${code.substring(code.length ~/ 2)}',
            style: TextStyle(
              fontSize: valueSize + 2,
              fontWeight: FontWeight.w600,
              letterSpacing: 1.5,
            ),
          ),
          actions: [
            // The seconds left, inside a ring that runs down with them.
            Semantics(
              label: '$left seconds left',
              excludeSemantics: true,
              child: SizedBox(
                width: 30,
                height: 30,
                child: Stack(
                  alignment: Alignment.center,
                  children: [
                    Positioned.fill(
                      child: CircularProgressIndicator(
                        value: left / period,
                        strokeWidth: 3,
                        backgroundColor: c.line,
                      ),
                    ),
                    Text(
                      '$left',
                      textScaler: TextScaler.noScaling,
                      style: TextStyle(
                        color: scheme.primary,
                        fontSize: 11,
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                  ],
                ),
              ),
            ),
            const SizedBox(width: 4),
            copyButton('one-time code', 'totp'),
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
            style: TextStyle(color: scheme.primary, fontSize: valueSize),
          ),
          actions: [
            if (!uri.startsWith('androidapp://') &&
                !uri.startsWith('iosapp://'))
              IconButton(
                tooltip: 'Open website',
                icon: Glyph(Glyphs.open, size: 20, color: c.muted),
                onPressed: () => openSite(uri),
              ),
            action('Copy website', copyGlyph, () {
              Clipboard.setData(ClipboardData(text: uri));
              showMessage(context, 'Copied the website');
            }),
          ],
        ),
      );
    }
    // A card's or an identity's details: plain ones copy as they are,
    // secrets reveal and copy like a password.
    void plainRow(String label, String? value, String what) {
      if (value == null || value.isEmpty) return;
      rows.add(
        row(
          label,
          SelectableText(value, style: TextStyle(fontSize: valueSize)),
          actions: [copyButton(label.toLowerCase(), what, secret: false)],
        ),
      );
    }

    final cardDetails = item['card'] as Map<String, dynamic>?;
    if (cardDetails != null) {
      plainRow(
        'Cardholder name',
        cardDetails['cardholderName'] as String?,
        'cardholderName',
      );
      final brand = cardDetails['brand'] as String?;
      if (brand != null && brand.isNotEmpty) {
        rows.add(
          row('Brand', Text(brand, style: TextStyle(fontSize: valueSize))),
        );
      }
      if (cardDetails['hasNumber'] == true) {
        rows.add(secretRow('Number', 'cardNumber'));
      }
      plainRow(
        'Expiration',
        cardDetails['expiration'] as String?,
        'expiration',
      );
      if (cardDetails['hasCode'] == true) {
        rows.add(secretRow('Security code', 'cardCode'));
      }
    }
    final sshKey = item['sshKey'] as Map<String, dynamic>?;
    if (sshKey != null) {
      plainRow('Public key', sshKey['publicKey'] as String?, 'sshPublicKey');
      plainRow(
        'Fingerprint',
        sshKey['fingerprint'] as String?,
        'sshFingerprint',
      );
      if (sshKey['hasPrivateKey'] == true) {
        rows.add(secretRow('Private key', 'sshPrivateKey'));
      }
    }
    final identityDetails = item['identity'] as Map<String, dynamic>?;
    if (identityDetails != null) {
      plainRow('Name', identityDetails['fullName'] as String?, 'fullName');
      plainRow(
        'Username',
        identityDetails['username'] as String?,
        'identityUsername',
      );
      plainRow('Company', identityDetails['company'] as String?, 'company');
      plainRow('Email', identityDetails['email'] as String?, 'email');
      plainRow('Phone', identityDetails['phone'] as String?, 'phone');
      plainRow('Address', identityDetails['address'] as String?, 'address');
      if (identityDetails['hasSsn'] == true) {
        rows.add(secretRow('Social Security number', 'ssn'));
      }
      if (identityDetails['hasPassportNumber'] == true) {
        rows.add(secretRow('Passport number', 'passportNumber'));
      }
      if (identityDetails['hasLicenseNumber'] == true) {
        rows.add(secretRow('License number', 'licenseNumber'));
      }
    }
    // A bank account's, driver's license's, or passport's values.
    for (final d
        in ((item['details'] as List?) ?? const [])
            .cast<Map<String, dynamic>>()) {
      if (d['secret'] == true) {
        rows.add(secretRow(d['label'] as String, d['key'] as String));
      } else {
        plainRow(
          d['label'] as String,
          d['value'] as String?,
          d['key'] as String,
        );
      }
    }
    final fields = (item['fields'] as List).cast<Map<String, dynamic>>();
    for (var i = 0; i < fields.length; i++) {
      final f = fields[i];
      // Our own data (fill rules, an expiration date) isn't a custom field.
      if (f['reserved'] == true) continue;
      if (f['type'] == 2) {
        rows.add(
          row(
            f['name'] as String,
            Text(
              f['value'] == 'true' ? 'Yes' : 'No',
              style: TextStyle(fontSize: valueSize),
            ),
          ),
        );
        continue;
      }
      if (f['type'] == 3) continue;
      rows.add(
        f['hidden'] == true
            ? secretRow(f['name'] as String, 'field:$i')
            : row(
                f['name'] as String,
                SelectableText(
                  (f['value'] ?? '') as String,
                  style: TextStyle(fontSize: valueSize),
                ),
                actions: [
                  copyButton(
                    (f['name'] as String).toLowerCase(),
                    'field:$i',
                    secret: false,
                  ),
                ],
              ),
      );
    }
    // Attachments: saved to Downloads, or to Files on an iPhone.
    for (final a
        in ((item['attachments'] as List?) ?? const [])
            .cast<Map<String, dynamic>>()) {
      rows.add(
        row(
          'Attachment',
          Text(
            '${a['name']}${a['size'] == null ? '' : ' · ${fileSize(a['size'] as num)}'}',
            style: TextStyle(fontSize: valueSize),
          ),
          // One whose name or key doesn't open is listed, but not saved.
          actions: [
            if (a['opens'] != false)
              action(
                'Save ${a['name']}',
                Icon(Icons.download_rounded, size: 20, color: scheme.primary),
                () => saveAttachment(a['id'] as String, a['name'] as String),
              ),
          ],
        ),
      );
    }
    // A login's passkey: who it's for and when it was made, never its key.
    final passkey = item['passkey'] as Map<String, dynamic>?;
    if (passkey != null) {
      final site = passkey['rpName'] as String? ?? passkey['rpId'] as String;
      final created = madeOn(passkey['created'] as String?);
      rows.add(
        row(
          'Passkey',
          Text(
            created == null ? 'For $site' : 'For $site · created $created',
            style: TextStyle(fontSize: valueSize),
          ),
        ),
      );
    }
    final expires = item['expires'] as String?;
    if (expires != null && expires.isNotEmpty) {
      // The date, and when that is from today.
      final expiry = item['expiry'] as Map?;
      final when = expiry?['date'] == expires ? expiry!['label'] as String? : null;
      rows.add(
        row(
          'Expires',
          Text(
            when == null ? expires : '$expires · $when',
            style: TextStyle(fontSize: valueSize),
          ),
        ),
      );
      // When it reminds, when that isn't the default (core/src/expiration.rs).
      final remind = item['remind'] as Map?;
      if (remind?['value'] != null) {
        rows.add(
          row(
            'Remind me',
            Text(
              remind!['words'] as String,
              style: TextStyle(fontSize: valueSize),
            ),
          ),
        );
      }
    }
    if (item['hasNotes'] == true) {
      rows.add(secretRow('Notes', 'notes', canCopy: false));
    }

    final kind = ItemKind.of(item['kind'] as String);
    // What it is: its template's name, if it came from one, else its kind's;
    // a custom template says so.
    final template = (item['template'] as Map?)?['name'] as String?;
    final definition = item['definition'] as Map?;
    final what = definition != null
        ? 'Template for ${definition['base'] == 'login' ? 'logins' : 'notes'}'
        : template ?? kind.one;
    final where = whereIs(item, widget.places);
    final jewel = item['category'] == null
        ? kind
        : Categories.of(item['category'] as String);
    // Someone whose shared folder hides passwords is told why they can't see them.
    if (!sees) {
      rows.add(
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
          child: Text(
            'Your organization hides this item\'s passwords from you.',
            style: TextStyle(color: c.muted, fontSize: 13),
          ),
        ),
      );
    }
    // Under the details, as the web vault's panel and the extension have
    // them: how strong the password is and when it changed, when the item
    // changed and was made, and its version history.
    final strength = item['strength'] as String?;
    final since =
        (item['passwordChanged'] as String?) ?? (item['created'] as String?);
    final healthLine = strength == null
        ? null
        : [
            'Password $strength',
            if (ago(since) case final at?)
              item['passwordChanged'] != null ? 'changed $at' : 'added $at',
          ].join(' · ');
    final dates = [
      if (madeOn(item['revisionDate'] as String?) case final d?) 'Changed $d',
      if (madeOn(item['created'] as String?) case final d?) 'created $d',
    ].join(' · ');
    final extras = <Widget>[
      if (healthLine != null)
        Padding(
          padding: const EdgeInsets.fromLTRB(4, 14, 4, 0),
          child: Row(
            children: [
              Glyph(
                Glyphs.health,
                size: 18,
                color: strength == 'weak' ? scheme.error : scheme.primary,
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  healthLine,
                  style: TextStyle(
                    color: strength == 'weak' ? scheme.error : c.muted,
                    fontSize: 13,
                  ),
                ),
              ),
            ],
          ),
        ),
      if (may(item, 'viewHistory'))
        Padding(
          padding: const EdgeInsets.only(top: 10),
          child: Align(
            alignment: Alignment.centerLeft,
            child: TextButton.icon(
              onPressed: history,
              icon: Glyph(Glyphs.history, size: 18, color: scheme.primary),
              label: const Text('Version history'),
            ),
          ),
        ),
      if (dates.isNotEmpty)
        Padding(
          padding: const EdgeInsets.fromLTRB(4, 4, 4, 0),
          child: Text(dates, style: TextStyle(color: c.muted, fontSize: 12)),
        ),
    ];
    final card = RestingCard(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (var i = 0; i < rows.length; i++) ...[
            if (i > 0) const Divider(indent: 16),
            rows[i],
          ],
        ],
      ),
    );
    if (phone) {
      return ListView(
        padding: const EdgeInsets.fromLTRB(16, 4, 16, 32),
        children: [
          // A big tile, its name, and what it is and where.
          Center(
            child: DecoratedBox(
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(20),
                boxShadow: c.lift,
              ),
              child: Avatar.of(item, size: 72),
            ),
          ),
          const SizedBox(height: 14),
          Text(
            item['name'] as String,
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 24, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 6),
          Text.rich(
            TextSpan(
              children: [
                WidgetSpan(
                  alignment: PlaceholderAlignment.middle,
                  child: Padding(
                    padding: const EdgeInsets.only(right: 6),
                    child: JewelTile(jewel: jewel, size: 18),
                  ),
                ),
                TextSpan(text: '$what in $where'),
              ],
            ),
            textAlign: TextAlign.center,
            style: TextStyle(color: c.muted, fontSize: 14),
          ),
          const SizedBox(height: 22),
          if (rows.isNotEmpty) card,
          ...extras,
        ],
      );
    }
    // On a computer: a raised card, as the web vault has it, with the
    // details in a well inside, under the item's actions (build).
    return ListView(
      padding: const EdgeInsets.fromLTRB(12, 4, 20, 20),
      children: [
        Align(
          alignment: Alignment.topLeft,
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 600),
            child: Container(
              padding: const EdgeInsets.all(24),
              decoration: c.lifted.decoration(BorderRadius.circular(16)),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      DecoratedBox(
                        decoration: BoxDecoration(
                          borderRadius: BorderRadius.circular(15),
                          boxShadow: c.lift,
                        ),
                        child: Avatar.of(item, size: 56),
                      ),
                      const SizedBox(width: 14),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(
                              item['name'] as String,
                              style: const TextStyle(
                                fontSize: 22,
                                fontWeight: FontWeight.w700,
                              ),
                            ),
                            const SizedBox(height: 2),
                            Text(
                              '$what · $where',
                              style: TextStyle(color: c.muted, fontSize: 13),
                            ),
                          ],
                        ),
                      ),
                    ],
                  ),
                  if (rows.isNotEmpty) ...[
                    const SizedBox(height: 18),
                    DecoratedBox(
                      decoration: BoxDecoration(
                        color: c.well,
                        borderRadius: BorderRadius.circular(12),
                        border: Border.all(color: c.wellEdge),
                      ),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          for (var i = 0; i < rows.length; i++) ...[
                            if (i > 0) Divider(indent: 16, color: c.wellEdge),
                            rows[i],
                          ],
                        ],
                      ),
                    ),
                  ],
                  ...extras,
                ],
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// Asks for the master password for an item that asks for it again
/// (docs/core.md, "Asking for the master password again"); true once it's
/// given.
Future<bool> askForMasterPassword(
  BuildContext context,
  String id,
  String name,
) async =>
    await showDialog<bool>(
      context: context,
      builder: (_) => _MasterPasswordDialog(id: id, name: name),
    ) ==
    true;

class _MasterPasswordDialog extends StatefulWidget {
  const _MasterPasswordDialog({required this.id, required this.name});

  final String id;
  final String name;

  @override
  State<_MasterPasswordDialog> createState() => _MasterPasswordDialogState();
}

class _MasterPasswordDialogState extends State<_MasterPasswordDialog> {
  final password = TextEditingController();
  String? error;
  bool working = false;

  @override
  void dispose() {
    password.dispose();
    super.dispose();
  }

  Future<void> confirm() async {
    if (password.text.isEmpty || working) return;
    setState(() {
      working = true;
      error = null;
    });
    final answer = await whileBusy(
      () => Core.confirmMasterPassword(widget.id, password.text),
    );
    if (!mounted) return;
    password.clear();
    if (answer.ok) {
      Navigator.of(context).pop(true);
      return;
    }
    setState(() {
      working = false;
      error = answer.error;
    });
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(widget.name),
    content: SizedBox(
      width: 360,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('This item asks for your master password.'),
          const SizedBox(height: 12),
          TextField(
            controller: password,
            obscureText: true,
            autofocus: true,
            decoration: InputDecoration(
              labelText: 'Master password',
              errorText: error,
            ),
            onSubmitted: (_) => confirm(),
          ),
        ],
      ),
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.of(context).pop(false),
        child: const Text('Cancel'),
      ),
      WorkingButton(label: 'Continue', working: working, onPressed: confirm),
    ],
  );
}

/// A file's size for people: "37 bytes", "1.2 KB", "3.4 MB".
String fileSize(num bytes) {
  if (bytes < 1000) return '${bytes.toInt()} bytes';
  if (bytes < 1000 * 1000) return '${(bytes / 1000).toStringAsFixed(1)} KB';
  return '${(bytes / (1000 * 1000)).toStringAsFixed(1)} MB';
}

/// A folder's Rename and Delete, beside its name in the sidebar, or (large)
/// in a phone's header while the folder is shown.
class FolderMenu extends StatelessWidget {
  const FolderMenu({
    super.key,
    required this.name,
    required this.color,
    required this.onRename,
    required this.onDelete,
    this.large = false,
  });

  final String name;
  final Color color;
  final VoidCallback onRename;
  final VoidCallback onDelete;
  final bool large;

  @override
  Widget build(BuildContext context) {
    final menu = PopupMenuButton<VoidCallback>(
      tooltip: 'More for $name',
      padding: large ? const EdgeInsets.all(8) : EdgeInsets.zero,
      iconSize: large ? 24 : 16,
      icon: Icon(
        large ? Icons.more_vert_rounded : Icons.more_horiz_rounded,
        color: color,
      ),
      onSelected: (action) => action(),
      itemBuilder: (context) => [
        PopupMenuItem(value: onRename, child: const Text('Rename')),
        PopupMenuItem(value: onDelete, child: const Text('Delete')),
      ],
    );
    return large ? menu : SizedBox(width: 24, height: 20, child: menu);
  }
}

/// A folder's name, asked for: null when cancelled.
Future<String?> askFolderName(
  BuildContext context,
  String title, [
  String name = '',
]) => showDialog<String>(
  context: context,
  builder: (context) => _FolderNameDialog(title: title, name: name),
);

class _FolderNameDialog extends StatefulWidget {
  const _FolderNameDialog({required this.title, required this.name});

  final String title;
  final String name;

  @override
  State<_FolderNameDialog> createState() => _FolderNameDialogState();
}

class _FolderNameDialogState extends State<_FolderNameDialog> {
  late final field = TextEditingController(text: widget.name);

  @override
  void dispose() {
    field.dispose();
    super.dispose();
  }

  void done() {
    final name = field.text.trim();
    if (name.isNotEmpty) Navigator.pop(context, name);
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(widget.title),
    content: TextField(
      controller: field,
      autofocus: true,
      decoration: const InputDecoration(labelText: 'Name'),
      onChanged: (_) => setState(() {}),
      onSubmitted: (_) => done(),
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.pop(context),
        child: const Text('Cancel'),
      ),
      FilledButton(
        onPressed: field.text.trim().isEmpty ? null : done,
        child: const Text('Save'),
      ),
    ],
  );
}

/// A date the server gave (ISO 8601) as people read it: "October 20, 2026".
String? madeOn(String? iso) {
  final at = iso == null ? null : DateTime.tryParse(iso)?.toLocal();
  if (at == null) return null;
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
