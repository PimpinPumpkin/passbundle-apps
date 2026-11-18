// The vault: a menu (all items, favorites, the categories, and the folders:
// Personal's, then each organization's shared folders), the list with
// search, and the item, side by side on a wide window, as the web vault has
// them. On a phone (or a phone-wide window), the items are a tab beside
// Settings: their kinds as colored tiles, then favorites, the folders, and
// every item, and an item opens on a page of its own. Copying goes through
// the core, so a password never passes through Dart; secrets reach the
// screen only while revealed, and never when the person's shared folder
// hides them. Several items can be selected in a list and moved, favorited,
// archived, or put in the trash together, and the trash restores them or
// deletes them for good (selection.dart). Above the list, someone no rule
// requires two-step login of is nudged to set it up, or to add something
// stronger than email codes (two_step_nudge.dart). On a phone, the phone says
// expiration reminders while PassBundle is closed too (closed_reminders.dart).
import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'accounts.dart';
import 'attaching.dart';
import 'autofill_offer.dart';
import 'browser_import.dart';
import 'closed_reminders.dart';
import 'core.dart';
import 'editor.dart';
import 'generator.dart';
import 'glyphs.dart';
import 'history.dart';
import 'icons.dart';
import 'new_item.dart';
import 'own_health.dart';
import 'places.dart';
import 'selection.dart';
import 'send.dart';
import 'settings.dart';
import 'sign_in_requests.dart';
import 'theme.dart';
import 'transfer.dart';
import 'two_step_nudge.dart';
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

/// The menu's width on a computer, and the room around it.
const sidebarWidth = 248.0;

/// The item's pane beside the list on a computer, in a window this wide.
double paneWidth(double window) => (window * 0.42).clamp(380, 552);

/// Where a notice floats on a computer: over the list's column, clear of
/// the menu (its Lock and Settings) and, where the list has room, the
/// item's pane.
EdgeInsets noticeInsets(double window) {
  const left = 12 + sidebarWidth + 16;
  final pane = paneWidth(window);
  final right = window - left - pane >= 360 ? pane + 8 : 12.0;
  return EdgeInsets.fromLTRB(left, 5, right, 12);
}

/// Settings asked for from outside the vault's screens (the Mac's PassBundle
/// menu, Settings…): the vault shown opens them.
final settingsAsked = ValueNotifier<int>(0);

/// A phone's reminder was tapped while PassBundle was open
/// (closed_reminders.dart): the vault shown, if it's unlocked, shows
/// Expiring soon. Locked, it does once it's unlocked.
final expiringAsked = ValueNotifier<int>(0);

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

  /// The item open beside the list, on a computer.
  String? selected;

  /// The items selected in the list, by key (`selectionKey`: an item's
  /// account and id, since "All accounts" lists several accounts' items), to
  /// do one thing to all of them (selection.dart): always among those shown.
  final checked = <String>{};

  /// Select is on (a computer's Select button): every row has its
  /// checkbox, and a click selects, even with nothing selected yet.
  bool selectMode = false;

  /// Where a range starts (Shift and a click): the item last selected, by
  /// key.
  String? anchor;

  /// Whether the selected items' bar offers the trash (in the trash,
  /// deleting for good), and restoring: not when the person may do it to
  /// none of them.
  bool trashOffered = true;
  bool restoreOffered = true;

  /// Counts the bulk actions that changed the open item, so its pane reads
  /// it again.
  int refreshed = 0;

  /// What the search finds (everything, without one).
  List<Map<String, dynamic>> all = [];

  /// Every item, whatever the search: the menu's counts.
  List<Map<String, dynamic>> everything = [];

  /// The archived items the search finds, for Archive, and how many there
  /// are in all, for its count.
  List<Map<String, dynamic>> archived = [];
  int archivedCount = 0;

  /// The items in the trash the search finds, for Trash, and how many there
  /// are in all, for its count.
  List<Map<String, dynamic>> trashed = [];
  int trashedCount = 0;

  /// Every unlocked account's items, with several accounts signed in ("All
  /// accounts"): what the search finds there while it's shown, else all.
  List<Map<String, dynamic>> everyAccount = [];

  /// More than one account is signed in here.
  bool get several => widget.menu?.several ?? false;
  Places places = Places.empty;

  /// The logins with something to fix, by id (core/src/health.rs): judged
  /// on a background isolate after each change.
  Map<String, Map<String, dynamic>> health = const {};

  /// The report Health shows (`weak`, `reused`, `unsecured`, `expiring`),
  /// or null for everything it has to fix.
  String? report;

  /// The organizations whose admins see how many of the person's own
  /// passwords are weak, reused, or exposed, by name, which Health says
  /// (own_health.dart); empty while none does.
  List<String> seenBy = const [];
  bool syncing = false;
  bool signedOut = false;

  /// This device couldn't reach the server at the last sync: how many
  /// seconds old the copy kept here is (0 when it isn't known), or null when
  /// it could. The vault says it shows that copy, instead of an error each
  /// time it opens.
  int? offline;

  /// Which nudge about two-step login shows above the list
  /// (two_step_nudge.dart): "setUp", "stronger", or null. This account's
  /// server's answer at the last sync.
  String? twoStepNudge;

  /// Set it up opened the web vault's two-step settings in the browser:
  /// coming back to the app syncs, so a method just set up there ends the
  /// nudge here.
  bool settingUpTwoStep = false;

  /// On a phone: the items (0), the generator (1), Send (2), or Settings
  /// (3).
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
    // Back from setting up two-step login in the browser: the vault syncs,
    // which asks the server about the nudge again.
    onResume: () {
      if (!settingUpTwoStep) return;
      settingUpTwoStep = false;
      unawaited(sync());
    },
  );

  /// A sign-in request open over the vault (sign_in_requests.dart): one at
  /// a time; the next waits for it.
  bool prompting = false;

  /// What's due to be said about items expiring ("2 items expire soon",
  /// core/src/expiration.rs), at the top of the vault until it's shown or
  /// dismissed. Asked for after unlocking and every six hours after. On a
  /// phone, what the phone said while PassBundle was closed isn't said again
  /// (closed_reminders.dart), and the phone's coming days are worked out again
  /// from what was said now.
  String? reminder;
  Timer? reminding;

  Future<void> remind() async {
    if (!mounted) return;
    final delivered = await remindersDelivered();
    if (!mounted) return;
    final answer = await Core.reminders(delivered);
    if (!mounted || !answer.ok) return;
    if (answer.body['title'] case final String title) {
      setState(() => reminder = title);
    }
    unawaited(scheduleClosedReminders());
  }

  /// Opened from a phone's reminder, or one tapped while open: Expiring
  /// soon, on the items' tab.
  Future<void> showExpiringIfAsked() async {
    if (!mounted || !await openedFromReminder() || !mounted) return;
    setState(() => tab = 0);
    choose(Filter.expiring);
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
    expiringAsked.addListener(showExpiringIfAsked);
    vaultChanged.addListener(reload);
    // Read now, before the first sync has the core.
    final offer = widget.offerBiometric && mayOfferBiometric(widget.core);
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      if (widget.syncOnOpen) sync();
      updateFound();
      // Expiration reminders, each said once (and on a computer, the
      // desktop agent's tray told how many); and opened from a phone's
      // reminder, Expiring soon.
      if (mounted) {
        unawaited(remind());
        reminding = Timer.periodic(const Duration(hours: 6), (_) => remind());
        unawaited(showExpiringIfAsked());
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
      // Then, on Android or an iPhone, turning on autofill, until it's on or
      // the person says "Later" (autofill_offer.dart).
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
      // Then, on a phone, the first time there's a reminder to schedule,
      // asking about reminders while PassBundle is closed.
      if (!saving && mounted) await offerClosedReminders(context);
      // Then, on a computer, importing from its browsers: offered once per
      // account here, when there's a browser profile to import from.
      if (!saving && !isPhone && widget.syncOnOpen && mounted) {
        await offerBrowserImport(
          context,
          email: widget.email,
          server: widget.server,
        );
        if (mounted) reload();
      }
      if (!mounted) return;
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
    expiringAsked.removeListener(showExpiringIfAsked);
    vaultChanged.removeListener(reload);
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

  /// A sign-in, a card, or an address Android's autofill offered to save:
  /// the editor, filled in. True if there was one. A new password for a
  /// login it has comes with that login's id and account, and opens in that
  /// account; anything new, with more than one account unlocked, asks which
  /// account it goes in. A card or an address the person has already isn't
  /// saved again, and goes only where an organization's policy doesn't
  /// remove its kind (core/src/restricted_types.rs).
  Future<bool> saveOffered() async {
    if (!mounted) return false;
    final save = await Core.pendingSave();
    if (save == null || !mounted) return false;
    final card = save['kind'] == 'card';
    if (save['already'] case final String name) {
      showMessage(
        context,
        card
            ? 'This card is in PassBundle already ($name).'
            : 'This address is in PassBundle already ($name).',
      );
      return true;
    }
    final mayAdd = (save['mayAdd'] as List?)?.cast<String>();
    if (mayAdd != null && mayAdd.isEmpty) {
      showMessage(
        context,
        'An organization you\'re in doesn\'t allow '
        '${card ? 'cards' : 'identities'}, so PassBundle didn\'t save it.',
      );
      return true;
    }
    final id = save['id'] as String?;
    var account = save['account'] as String? ?? Core.account;
    if (id == null && widget.menu != null) {
      final chosen = await chooseAccount(context, widget.menu!, among: mayAdd);
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
      final allArchived = widget.core.archived('');
      archived = searching.isEmpty ? allArchived : widget.core.archived(searching);
      archivedCount = allArchived.length;
      final allTrashed = widget.core.trashed('');
      trashed = searching.isEmpty ? allTrashed : widget.core.trashed(searching);
      trashedCount = allTrashed.length;
      places = Places(widget.core.places());
      seenBy = widget.core.ownHealthSeenBy();
      if (selected != null && widget.core.item(selected!) == null) {
        selected = null;
      }
      keepChecked();
    });
    judge();
    // On a phone, the reminders it says while PassBundle is closed, from the
    // items as they are now.
    unawaited(scheduleClosedReminders());
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
      keepChecked();
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
      // Offline, signed out, or a server without it: no nudge.
      twoStepNudge = answer.ok ? twoStepNudgeOf(answer.body) : null;
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
    for (final i in switch (filter) {
      Filter.everyAccount => everyAccount,
      Filter.archive => archived,
      Filter.trash => trashed,
      _ => all,
    })
      if (shows(i, filter, filterId, health: health) &&
          (filter != Filter.health ||
              report == null ||
              inReport(health[i['id']]!, report!)))
        // Health says what each login has to fix; when it expires is its
        // badge's to say, so it isn't said twice.
        if (filter == Filter.health)
          {
            ...i,
            'subtitle': switch (healthIssues(
              health[i['id']]!,
              expiry: expiringLabel(i) == null,
            )) {
              '' => i['subtitle'],
              final issues => issues,
            },
          }
        else
          i,
  ];

  /// Nothing's in this account's vault yet, and nothing narrows the list:
  /// the list says what to do first instead.
  bool get emptyVault =>
      everything.isEmpty &&
      query.isEmpty &&
      (filter == Filter.all || filter == Filter.everyAccount) &&
      (filter != Filter.everyAccount || everyAccount.isEmpty);

  /// What an empty vault offers: New, and the imports.
  Widget emptyVaultNote() => EmptyVault(
    onNew: newItem,
    onBrowsers: isPhone ? null : importFromBrowsers,
    onImport: importFile,
  );

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
    Filter.archive => 'Archive',
    Filter.trash => 'Trash',
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
      Filter.archive =>
        'Nothing in Archive. An archived item stays in your vault, out of the lists, search, and filling.',
      Filter.trash =>
        'Nothing in the trash. An item moved to the trash waits here until you restore it or delete it forever.',
      _ => 'Nothing here yet.',
    };
  }

  void choose(Filter f, [String? id]) {
    final searchesChange = (filter == Filter.everyAccount) !=
        (f == Filter.everyAccount);
    setState(() {
      filter = f;
      filterId = id;
      // Health opens on everything it has to fix.
      report = null;
      // What's selected is of the list shown.
      checked.clear();
      selectMode = false;
      anchor = null;
    });
    // The search is every account's there, and this one's elsewhere.
    if (searchesChange && query.isNotEmpty) reload();
  }

  /// Over Health's list on a computer: its reports, and who sees how many of
  /// the person's own passwords are weak, reused, or exposed (own_health.dart).
  Widget get healthHeader => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    mainAxisSize: MainAxisSize.min,
    children: [
      if (health.isNotEmpty) healthReportsBar,
      if (health.isNotEmpty && seenBy.isNotEmpty) const SizedBox(height: 10),
      if (seenBy.isNotEmpty) SeenByLine(seenBy),
    ],
  );

  /// Health's reports, each with how many items, as a row of choices over
  /// its list: picking one shows only its items.
  Widget get healthReportsBar => HealthReports(
    health: health,
    report: report,
    onReport: (picked) => setState(() {
      report = picked;
      checked.clear();
      anchor = null;
    }),
  );

  /// The selected items, in the list's order.
  List<Map<String, dynamic>> get checkedItems => [
    for (final i in shown)
      if (checked.contains(selectionKey(i))) i,
  ];

  /// The selected items by account, each account's in the list's order:
  /// "All accounts" acts on each item in its own account.
  Map<String, List<String>> get checkedByAccount =>
      byAccount([for (final i in checkedItems) selectionKey(i)]);

  /// The selection stays among the items shown: one a search or a sync
  /// took out of the list isn't selected any more, so nothing is done to an
  /// item the person can't see. Inside a setState.
  void keepChecked() {
    if (checked.isEmpty) return;
    final here = {for (final i in shown) selectionKey(i)};
    checked.retainAll(here);
    if (!here.contains(anchor)) anchor = null;
    offer();
  }

  /// What the selected items' bar offers, once the selection changes. The
  /// trash isn't offered when the person may delete none of them, nor
  /// restoring and deleting for good in the trash when they may do neither,
  /// which the core says (`Core.bulkPlan`, nothing sent); while a slow call
  /// runs it isn't asked, and the question before it says so instead.
  void offer() {
    trashOffered = true;
    restoreOffered = true;
    if (checked.isEmpty || busy.value) return;
    final ids = checkedByAccount;
    bool offered(BulkAction action) {
      final plan = planAcross(widget.core, ids, action.json);
      return !plan.ok ||
          (plan.body['go'] as List).isNotEmpty ||
          leftBy(plan).containsKey('masterPassword');
    }

    if (filter == Filter.trash) {
      restoreOffered = offered(const BulkAction.restore());
      trashOffered = offered(const BulkAction.deleteForever());
    } else {
      trashOffered = offered(const BulkAction.trash());
    }
  }

  /// Selects an item, or takes it out of the selection.
  void check(Map<String, dynamic> item) {
    final key = selectionKey(item);
    setState(() {
      if (!checked.remove(key)) checked.add(key);
      anchor = checked.contains(key) ? key : null;
      offer();
    });
  }

  /// Selects every item from the last one selected to this one (Shift and
  /// a click); with none to start from, this one.
  void checkTo(Map<String, dynamic> item) {
    final list = shown;
    final from = list.indexWhere((i) => selectionKey(i) == anchor);
    final to = list.indexWhere((i) => selectionKey(i) == selectionKey(item));
    if (anchor == null || from < 0 || to < 0) {
      if (!checked.contains(selectionKey(item))) check(item);
      return;
    }
    setState(() {
      for (var i = math.min(from, to); i <= math.max(from, to); i++) {
        checked.add(selectionKey(list[i]));
      }
      offer();
    });
  }

  /// Selects everything shown.
  void checkAll() => setState(() {
    checked.addAll([for (final i in shown) selectionKey(i)]);
    offer();
  });

  /// Lets go of everything selected, and Select.
  void uncheckAll() {
    if (checked.isEmpty && !selectMode) return;
    setState(() {
      checked.clear();
      selectMode = false;
      anchor = null;
    });
  }

  /// Select, on a computer: every row's checkbox, and a click selects.
  void startSelecting() => setState(() => selectMode = true);

  /// Every selected item is a favorite: the star takes them out.
  bool get checkedAreFavorites =>
      checkedItems.every((i) => i['favorite'] == true);

  /// The account every selected item is in, or null when they're in more
  /// than one ("All accounts").
  String? get checkedAccount {
    final accounts = checkedByAccount.keys;
    return accounts.length == 1 ? accounts.single : null;
  }

  /// Where an account's items can go: this account's places, or another's
  /// ("All accounts").
  Places placesOf(String account) =>
      account == Core.account ? places : Places(widget.core.places(account));

  /// Whether Move has anywhere to offer: only when every selected item is
  /// in one account, a folder for the person's own items, or a shared
  /// folder the person may add items to, in an organization that has one
  /// of them.
  bool get moveOffered {
    final account = checkedAccount;
    if (account == null) return false;
    final where = placesOf(account);
    return checkedItems.any((i) {
      final owner = i['organizationId'] as String?;
      return owner == null || where.canCreateIn(owner);
    });
  }

  /// Move, for the selected items: asks where, then does it.
  Future<void> moveChecked() async {
    final items = checkedItems;
    final account = checkedAccount;
    if (items.isEmpty || account == null) return;
    final action = await chooseWhereToMove(
      context,
      places: placesOf(account),
      count: items.length,
      personal: items.any((i) => i['organizationId'] == null),
      organizations: {
        for (final i in items)
          if (i['organizationId'] case final String owner) owner,
      },
      here: filter == Filter.folder || filter == Filter.collection
          ? filterId
          : null,
      inNoFolder: filter == Filter.noFolder,
    );
    if (action == null || !mounted) return;
    await doToChecked(action);
  }

  /// What the selected items' bar offers, in the place shown: in the trash,
  /// restoring and deleting for good; elsewhere favorites, Move, Archive (or
  /// Move out of Archive in Archive), and the trash. Each only where the
  /// person may do it to any of them.
  List<BarAction> barActions({required bool phone}) {
    final size = phone ? 22.0 : 20.0;
    if (filter == Filter.trash) {
      return [
        if (restoreOffered)
          BarAction(
            'Restore',
            const Icon(Icons.restore_from_trash_rounded, size: 22),
            () => doToChecked(const BulkAction.restore()),
          ),
        if (trashOffered)
          BarAction(
            'Delete forever',
            const Icon(Icons.delete_forever_rounded, size: 22),
            () => doToChecked(const BulkAction.deleteForever()),
          ),
      ];
    }
    final favorites = checkedAreFavorites;
    final inArchive = filter == Filter.archive;
    return [
      BarAction(
        favorites ? 'Remove from favorites' : 'Add to favorites',
        Glyph(
          Glyphs.star,
          size: size,
          color: favorites
              ? favoriteEdge
              : (phone ? null : context.colors.muted),
          fill: favorites ? favoriteColor : null,
          strokeWidth: 1.6,
        ),
        () => doToChecked(BulkAction.favorite(!favorites)),
      ),
      if (moveOffered)
        BarAction('Move', Glyph(Glyphs.folder, size: size), moveChecked),
      BarAction(
        inArchive ? 'Move out of Archive' : 'Archive',
        Glyph(Glyphs.archive, size: size),
        () => doToChecked(BulkAction.archive(!inArchive)),
      ),
      if (trashOffered)
        BarAction(
          'Move to trash',
          const Icon(Icons.delete_outline_rounded, size: 22),
          () => doToChecked(const BulkAction.trash()),
        ),
    ];
  }

  /// Does one thing to every selected item (selection.dart), each in its
  /// own account. The core says first which it may be done to
  /// (`Core.bulkPlan`): the trash and deleting for good always ask, with
  /// how many; anything else asks only when some items would be left as
  /// they are, saying how many and why, or ask for the master password.
  /// What happened is said after, and an item it failed for stays selected.
  Future<void> doToChecked(BulkAction action) async {
    final ids = checkedByAccount;
    if (ids.isEmpty) return;
    final plan = planAcross(widget.core, ids, action.json);
    if (!plan.ok) {
      showMessage(context, plan.error ?? 'Couldn\'t do that.');
      return;
    }
    final asks =
        action.alwaysAsks || (plan.body['skipped'] as List).isNotEmpty;
    final passwordFor = askingIn(plan);
    final answer = asks
        ? await confirmBulk(
            context,
            action: action,
            ids: ids,
            plan: plan,
            passwordOf: passwordFor == null || passwordFor == Core.account
                ? null
                : widget.menu?.entry(passwordFor)?.label,
          )
        : await whileBusy(() => bulkAcross(ids, action.json));
    if (answer == null || !mounted) return;
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t do that.', inPlace: true);
      reload();
      return;
    }
    final outcome = BulkOutcome(action, answer);
    final open = selected == null ? null : keyOf(Core.account, selected!);
    setState(() {
      checked
        ..clear()
        ..addAll(outcome.failed);
      selectMode = false;
      anchor = null;
      if (outcome.done.contains(open)) {
        if (action.leavesTheList) selected = null;
        refreshed++;
      }
    });
    reload();
    if (outcome.failed.isNotEmpty) {
      await showBulkFailure(context, outcome);
      return;
    }
    // Only this account's items are in its trash.
    final allHere = outcome.done.every((k) => splitKey(k).$1 == Core.account);
    showMessage(
      context,
      outcome.said,
      // Longer to read with what was left and why.
      seconds: outcome.left.isEmpty && outcome.synced ? null : 8,
      // What the last one did isn't waited out first.
      inPlace: true,
      action: outcome.done.isEmpty
          ? null
          : switch (action.kind) {
              BulkKind.trash when allHere && filter != Filter.trash => (
                'Open trash',
                () => choose(Filter.trash),
              ),
              BulkKind.archive || BulkKind.unarchive => (
                'Undo',
                () => undoArchive(outcome),
              ),
              _ => null,
            },
    );
  }

  /// Takes what a bulk Archive or Move out of Archive did back: the same
  /// items, each in its own account, the other way.
  Future<void> undoArchive(BulkOutcome outcome) async {
    final back = BulkAction.archive(outcome.action.kind != BulkKind.archive);
    final answer = await whileBusy(
      () => bulkAcross(byAccount(outcome.done), back.json),
    );
    if (!mounted) return;
    reload();
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t do that.', inPlace: true);
      return;
    }
    final undone = BulkOutcome(back, answer);
    showMessage(context, undone.said, inPlace: true);
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

  /// New item: from "All accounts", which account it goes in, with more
  /// than one unlocked (the chosen account's vault opens to make it there);
  /// anywhere else it's the account shown's, which the menu names. Then the
  /// picker (the main kinds, then templates), then its editor; or, where a
  /// new item has nowhere to go ([nowhereToAdd]), only the words.
  Future<void> newItem({bool ask = true}) async {
    if (ask && widget.menu != null && filter == Filter.everyAccount) {
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
    if (!mounted || nowhereToAdd()) return;
    final wide = isWide(context);
    final choice = await chooseNewItem(
      context,
      templates: widget.core.templates(),
      asDialog: wide,
      onWebVault: widget.server == null ? null : () => openWebVault('/#/vault'),
      restricted: widget.core.restrictedKinds(),
    );
    if (choice == null || !mounted) return;
    await edit(null, null, choice);
  }

  /// Where an organization keeps items out of Personal and the person has
  /// no shared folder they can add to, a new item has nowhere to go
  /// (core/src/app.rs, `new_item_places`): the vault says so, in the
  /// import's words, and opens no editor. True then.
  bool nowhereToAdd() {
    if (!places.nowhereNew) return false;
    showMessage(context, places.nowhereWords, seconds: 8);
    return true;
  }

  Future<void> edit([
    String? editId,
    Map<String, dynamic>? prefill,
    NewItemChoice? choice,
  ]) async {
    // A new password Android's autofill offered to save for one of the
    // person's own logins, where an organization keeps items out of
    // Personal: the server refuses a change to it (`keptOutBy`), so it
    // goes in a new login, where a new item goes.
    final keptOut =
        editId != null &&
        prefill != null &&
        widget.core.item(editId)?['keptOutBy'] != null;
    final id = keptOut ? null : editId;
    if (id == null && nowhereToAdd()) return;
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

  /// The generator on its own, in a dialog (a computer's Tools), as tall
  /// as what's in it.
  Future<void> openGeneratorTool() => showDialog<void>(
    context: context,
    builder: (context) => Dialog(
      clipBehavior: Clip.antiAlias,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 480, maxHeight: 760),
        child: GeneratorPage(core: widget.core, title: false),
      ),
    ),
  );

  /// Send, in a dialog (a computer's Tools; send.dart).
  Future<void> openSendTool() => showDialog<void>(
    context: context,
    builder: (context) => Dialog(
      clipBehavior: Clip.antiAlias,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 560, maxHeight: 720),
        child: Scaffold(
          appBar: AppBar(
            title: const Text('Send'),
            automaticallyImplyLeading: false,
            leading: IconButton(
              tooltip: 'Close',
              icon: const Icon(Icons.close_rounded),
              onPressed: () => Navigator.of(context).pop(),
            ),
          ),
          body: SendPage(core: widget.core, title: false),
        ),
      ),
    ),
  );

  /// Importing the passwords saved in this computer's browsers
  /// (browser_import.dart); the vault shows what came in.
  Future<void> importFromBrowsers() async {
    await showBrowserImport(
      context,
      email: widget.email,
      server: widget.server,
    );
    if (mounted) reload();
  }

  /// Importing a file (transfer.dart), [format] chosen to begin with; the
  /// vault shows what came in.
  Future<void> importFile([String? format]) =>
      showImport(context, format: format, server: widget.server);

  /// Exporting the person's own items to a file (transfer.dart).
  Future<void> exportFile() async {
    final said = await showExport(context);
    if (said != null && mounted) showMessage(context, said);
  }

  /// Archives an item, or takes it out of the archive (DESIGN.md in the
  /// meta repo, "Archive, export, and import in the apps"): it leaves the
  /// lists and search, or comes back, with Undo.
  /// True if it was done.
  Future<bool> archive(String id, String name, bool on) async {
    final answer = await whileBusy(() => Core.setArchived(id, on));
    if (!mounted) return false;
    if (!answer.ok) {
      showMessage(
        context,
        answer.error ??
            (on ? 'Couldn\'t archive it.' : 'Couldn\'t move it out of Archive.'),
      );
      return false;
    }
    setState(() {
      selected = null;
      refreshed++;
    });
    reload();
    // In place of the notice showing: an earlier archive's Undo is stale
    // once this one is done.
    showMessage(
      context,
      on ? 'Moved $name to Archive.' : 'Moved $name out of Archive.',
      action: ('Undo', () => archive(id, name, !on)),
      inPlace: true,
    );
    return true;
  }

  /// The web vault's pages (New item's kinds the app doesn't make yet): each
  /// opens there, at [path] ("/#/vault").
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
      setState(() {
        tab = 3;
        checked.clear();
        anchor = null;
      });
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
        content: const Text('You can restore it from the trash.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            style: dangerButton(context),
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
    // The trash, a tap away, where it's restored.
    showMessage(
      context,
      'Moved $name to the trash.',
      action: ('Open trash', () => choose(Filter.trash)),
    );
    return true;
  }

  /// Restores an item from the trash, back where it was (an archived one to
  /// Archive): true if it went. The trash's own call, for one item.
  Future<bool> restore(String id, String name) =>
      doToOne(id, const BulkAction.restore(), 'Restored $name.');

  /// Deletes an item in the trash for good, once the person says so, which
  /// can't be undone: true if it went.
  Future<bool> deleteForever(String id, String name) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Delete $name forever?'),
        content: const Text(
          'It\'s deleted for good, with its attachments and history. This '
          'can\'t be undone.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            style: dangerButton(context),
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Delete forever'),
          ),
        ],
      ),
    );
    if (sure != true || !mounted) return false;
    return doToOne(
      id,
      const BulkAction.deleteForever(),
      'Deleted $name forever.',
    );
  }

  /// Does [action] to one item, as for several (the core's checks and the
  /// server's call), and says [said] once it's done, or why it wasn't.
  Future<bool> doToOne(String id, BulkAction action, String said) async {
    final answer = await whileBusy(() => Core.bulk([id], action.json));
    if (!mounted) return false;
    final went =
        answer.ok && ((answer.body['done'] as List?) ?? const []).contains(id);
    if (!went) {
      final left = leftBy(answer);
      showMessage(
        context,
        answer.error ??
            (left.isEmpty
                ? '${action.cant(1)}.'
                : '${action.cant(1)}: ${action.why(left.keys.first, 1)}.'),
      );
      reload();
      return false;
    }
    setState(() {
      selected = null;
      refreshed++;
    });
    reload();
    showMessage(context, said, inPlace: true);
    return true;
  }

  /// Makes a folder where the person says, as the web vault's New folder
  /// asks: in Personal, or in an organization where they may make shared
  /// folders, at the top or inside another one. Then shows it.
  Future<void> newFolder() async {
    final made = await askNewFolder(context, places);
    if (made == null || !mounted) return;
    final organization = made.organization;
    final answer = await whileBusy(
      () => organization == null
          ? Core.createFolder(made.name)
          : Core.createCollection(organization, made.name),
    );
    if (!mounted) return;
    reload();
    if (!answer.ok) {
      showMessage(context, answer.error ?? 'Couldn\'t make the folder.');
      return;
    }
    final id = answer.body['id'] as String?;
    if (organization == null) {
      choose(Filter.folder, id);
    } else if (places.collection(id) != null) {
      choose(Filter.collection, id);
    } else {
      choose(Filter.organization, organization);
    }
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
        title: Text('Delete the $name folder?'),
        content: const Text('Its items stay in Personal, in no folder.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            style: dangerButton(context),
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
        atRisk: atRisk(health[id]),
        onEdit: () => edit(id),
        onChanged: reload,
        onTrash: (name) async {
          if (await trash(id, name) && mounted) Navigator.of(context).pop();
        },
        onArchive: (name, on) async {
          if (await archive(id, name, on) && mounted) {
            Navigator.of(context).pop();
          }
        },
        onRestore: (name) async {
          if (await restore(id, name) && mounted) Navigator.of(context).pop();
        },
        onDeleteForever: (name) async {
          if (await deleteForever(id, name) && mounted) {
            Navigator.of(context).pop();
          }
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

  /// The nudge about two-step login, above the list (two_step_nudge.dart):
  /// this account's, so none in "All accounts", which lists several
  /// accounts' items.
  Widget? get twoStepNudgeBanner {
    final nudge = twoStepNudge;
    if (nudge == null || filter == Filter.everyAccount) return null;
    return TwoStepNudgeBanner(
      nudge: nudge,
      onSetUp: setUpTwoStep,
      onNotNow: notNowTwoStep,
    );
  }

  /// Set it up: the web vault's two-step login settings, in the browser.
  Future<void> setUpTwoStep() async {
    final server = widget.server;
    // Before the browser takes the screen, so coming back is heard.
    settingUpTwoStep = true;
    final opened =
        server != null && await Core.openWebsite('$server$webVaultTwoStepPath');
    if (!opened) {
      settingUpTwoStep = false;
      if (mounted) showMessage(context, 'Couldn\'t open the web vault.');
    }
  }

  /// Not now: hidden here at once, and on every device for 90 days once the
  /// server has it. If the server can't be told, the nudge comes back at a
  /// later sync; nothing waits on it.
  Future<void> notNowTwoStep() async {
    setState(() => twoStepNudge = null);
    await whileBusy(Core.dismissTwoStepNudge);
  }

  /// The banners under the list's search: signed out or offline, what's
  /// expiring, and the nudge about two-step login.
  Widget? get banners {
    final showing = [
      ?signedOutBanner,
      ?offlineBanner,
      ?reminderBanner,
      ?twoStepNudgeBanner,
    ];
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
            // Read again once a bulk action changes it.
            key: ValueKey((selected, refreshed)),
            core: widget.core,
            id: selected!,
            places: places,
            server: widget.server,
            atRisk: atRisk(health[selected]),
            onEdit: () => edit(selected),
            onChanged: reload,
            onTrash: (name) => trash(selected!, name),
            onArchive: (name, on) => archive(selected!, name, on),
            onRestore: (name) => restore(selected!, name),
            onDeleteForever: (name) => deleteForever(selected!, name),
          );
    // The top band is the window's (window_chrome.dart): the menu's top, the
    // list's search, and the item's actions sit where a title bar would, so
    // the signed-out banner goes in the list, under its search. Notices float
    // over the list's column, clear of the menu's Lock and Settings.
    final theme = Theme.of(context);
    return Theme(
      data: theme.copyWith(
        snackBarTheme: theme.snackBarTheme.copyWith(
          insetPadding: noticeInsets(MediaQuery.sizeOf(context).width),
        ),
      ),
      child: Scaffold(
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
                  onSend: openSendTool,
                  onBrowsers: isPhone ? null : importFromBrowsers,
                  onImport: importFile,
                  onExport: exportFile,
                  archived: archivedCount,
                  trashed: trashedCount,
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
                    emptyState: emptyVault ? emptyVaultNote() : null,
                    header:
                        filter == Filter.health &&
                            (health.isNotEmpty || seenBy.isNotEmpty)
                        ? healthHeader
                        : null,
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
                    checked: checked,
                    selectMode: selectMode,
                    onSelectMode: startSelecting,
                    onCheck: check,
                    onCheckTo: checkTo,
                    onCheckAll: checkAll,
                    onUncheckAll: uncheckAll,
                    selection: checked.isEmpty && !selectMode
                        ? null
                        : SelectionBar(
                            count: checked.length,
                            onClear: uncheckAll,
                            onSelectAll: checkAll,
                            actions: barActions(phone: false),
                          ),
                  ),
                ),
                // The item, beside the list, as the web vault's side panel:
                // up to 520 wide, the list taking the rest.
                SizedBox(
                  width: paneWidth(MediaQuery.sizeOf(context).width),
                  // Nothing to choose from, nothing said.
                  child:
                      pane ??
                      (shown.isEmpty
                          ? const SizedBox.shrink()
                          : Center(
                              child: Text(
                                'Choose an item',
                                style: TextStyle(color: context.colors.muted),
                              ),
                            )),
                ),
              ],
            ),
          ),
        ],
      ),
      ),
    );
  }

  /// On a phone: the vault and Settings, as tabs, and New item over the
  /// vault. Back goes back from a search, a kind or folder, or Settings,
  /// before it leaves the app; with items selected, it clears the selection
  /// first. While any are selected, what can be done with them is where the
  /// tabs are.
  Widget phone(BuildContext context) {
    final selecting = tab == 0 && checked.isNotEmpty;
    final home =
        tab == 0 && filter == Filter.all && query.isEmpty && !selecting;
    return PopScope(
      canPop: home,
      onPopInvokedWithResult: (popped, _) {
        if (popped) return;
        if (selecting) {
          uncheckAll();
        } else if (tab != 0) {
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
                        ? SendPage(core: widget.core)
                        : const SizedBox.shrink(),
                    tab == 3
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
        floatingActionButton: tab == 0 && !selecting
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
        bottomNavigationBar: selecting
            ? PhoneSelectionBar(actions: barActions(phone: true))
            : NavigationBar(
                selectedIndex: tab,
                onDestinationSelected: (i) => setState(() => tab = i),
                // The extension's: all items, the generator, Send, and the
                // gear.
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
                    icon: Glyph(Glyphs.send, size: 22),
                    label: 'Send',
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
        if (checked.isNotEmpty)
          // While items are selected: how many, with a way out and all.
          PageTitle(
            '${checked.length} selected',
            leading: IconButton(
              tooltip: 'Clear the selection',
              icon: const Icon(Icons.close_rounded),
              onPressed: uncheckAll,
            ),
            actions: [
              TextButton(onPressed: checkAll, child: const Text('Select all')),
              const SizedBox(width: 4),
            ],
          )
        else
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
        // The nudge about two-step login, above the list (on the Items tab
        // only).
        ?twoStepNudgeBanner,
        Expanded(
          child: RefreshIndicator(
            onRefresh: sync,
            child: CustomScrollView(
              key: const ValueKey('vault-scroll'),
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
    int count(Filter f, [String? id]) =>
        all.where((i) => shows(i, f, id)).length;
    if (filter != Filter.all || query.isNotEmpty) {
      final list = shown;
      // A folder's page lists the folders inside it first, as the menu
      // nests them (not while searching).
      final inside = query.isNotEmpty
          ? const <PlaceEntry>[]
          : [
              for (final e in PlaceEntry.all(places))
                if (e.id != null &&
                    !e.owner &&
                    (filter == Filter.folder || filter == Filter.collection) &&
                    e.filter == filter &&
                    (filter == Filter.folder
                            ? places.folder(e.id)
                            : places.collection(e.id))?['parentId'] ==
                        filterId)
                  PlaceEntry(
                    label: e.label,
                    glyph: e.glyph,
                    depth: 1,
                    filter: e.filter,
                    id: e.id,
                    ownFolder: e.ownFolder,
                    look: e.look,
                  ),
            ];
      return [
        if (filter == Filter.health && health.isNotEmpty && query.isEmpty)
          SliverPadding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
            sliver: SliverToBoxAdapter(child: healthReportsBar),
          ),
        // Who sees how many of the person's own passwords are weak, reused,
        // or exposed, as on a computer (own_health.dart).
        if (filter == Filter.health && seenBy.isNotEmpty && query.isEmpty)
          SliverPadding(
            padding: const EdgeInsets.fromLTRB(20, 8, 20, 4),
            sliver: SliverToBoxAdapter(child: SeenByLine(seenBy)),
          ),
        if (inside.isNotEmpty) ...[
          const SliverToBoxAdapter(child: SizedBox(height: 8)),
          placeCard([
            for (final e in inside)
              (e, count(e.filter, e.id), () => choose(e.filter, e.id)),
          ]),
        ],
        if (list.isEmpty && inside.isEmpty)
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
        else if (list.isNotEmpty) ...[
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
    // Each owner on a card of its own: Personal and its folders, then each
    // organization and its shared folders.
    final owners = <List<PlaceEntry>>[];
    for (final e in PlaceEntry.all(places)) {
      if (e.owner) owners.add([]);
      owners.last.add(e);
    }
    return [
      SliverPadding(
        padding: const EdgeInsets.only(top: 8),
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
      // Health, as a computer's menu has it: what to fix, by report.
      if (all.isNotEmpty) ...[
        const SliverToBoxAdapter(child: SizedBox(height: 12)),
        healthCard(),
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
      // Archive, then the trash, after the folders as there.
      const SliverToBoxAdapter(child: SizedBox(height: 10)),
      archiveCard(),
      const SliverToBoxAdapter(child: SizedBox(height: 10)),
      trashCard(),
      heading('All items'),
      if (listed.isEmpty)
        SliverToBoxAdapter(
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
            child: emptyVault
                ? emptyVaultNote()
                : Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 16,
                      vertical: 8,
                    ),
                    child: Text(emptyNote, style: TextStyle(color: c.muted)),
                  ),
          ),
        )
      else
        itemCard(listed),
      end,
    ];
  }

  /// Archive, on a phone: a row beside the trash, with how many it holds.
  Widget archiveCard() => card(1, (context, i, radius) {
    final c = context.colors;
    return InkWell(
      key: const ValueKey('archive'),
      borderRadius: radius,
      onTap: () => choose(Filter.archive),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 56),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 12, 8),
          child: Row(
            children: [
              Container(
                width: 40,
                height: 40,
                decoration: BoxDecoration(
                  color: c.tint,
                  borderRadius: BorderRadius.circular(11),
                ),
                alignment: Alignment.center,
                child: Glyph(
                  Glyphs.archive,
                  size: 21,
                  color: Theme.of(context).colorScheme.primary,
                ),
              ),
              const SizedBox(width: 12),
              const Expanded(
                child: Text(
                  'Archive',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(fontSize: 16, fontWeight: FontWeight.w500),
                ),
              ),
              Text('$archivedCount', style: TextStyle(color: c.muted, fontSize: 14)),
              const SizedBox(width: 6),
              Glyph(Glyphs.chevron, size: 18, color: c.muted, strokeWidth: 2),
            ],
          ),
        ),
      ),
    );
  });

  /// The trash, on a phone: a row beside Archive, with how many it holds.
  Widget trashCard() => card(1, (context, i, radius) {
    final c = context.colors;
    return InkWell(
      key: const ValueKey('trash'),
      borderRadius: radius,
      onTap: () => choose(Filter.trash),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 56),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 12, 8),
          child: Row(
            children: [
              Container(
                width: 40,
                height: 40,
                decoration: BoxDecoration(
                  color: c.tint,
                  borderRadius: BorderRadius.circular(11),
                ),
                alignment: Alignment.center,
                child: Glyph(
                  Glyphs.trash,
                  size: 21,
                  color: Theme.of(context).colorScheme.primary,
                ),
              ),
              const SizedBox(width: 12),
              const Expanded(
                child: Text(
                  'Trash',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(fontSize: 16, fontWeight: FontWeight.w500),
                ),
              ),
              Text('$trashedCount', style: TextStyle(color: c.muted, fontSize: 14)),
              const SizedBox(width: 6),
              Glyph(Glyphs.chevron, size: 18, color: c.muted, strokeWidth: 2),
            ],
          ),
        ),
      ),
    );
  });

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
        // A long press starts selecting; from then a tap selects, or
        // takes the item out. In "All accounts", each item in its own
        // account.
        final selecting = checked.isNotEmpty;
        return PhoneItemRow(
          item: item,
          radius: radius,
          badge: account == null ? null : AccountBadge(entry: account),
          checked: selecting ? checked.contains(selectionKey(item)) : null,
          onTap: selecting ? () => check(item) : () => pick(item),
          onLongPress: () => check(item),
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

  /// Health, on a phone: a card of its own, with how many items have
  /// something to fix in a topaz pill, as a computer's menu has it.
  Widget healthCard() => card(1, (context, i, radius) {
    final brightness = Theme.of(context).brightness;
    final c = context.colors;
    return InkWell(
      key: const ValueKey('health'),
      borderRadius: radius,
      onTap: () => choose(Filter.health),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 56),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 12, 8),
          child: Row(
            children: [
              const JewelTile(jewel: Jewel.health, size: 40),
              const SizedBox(width: 12),
              const Expanded(
                child: Text(
                  'Health',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(fontSize: 16, fontWeight: FontWeight.w700),
                ),
              ),
              if (health.isNotEmpty)
                DecoratedBox(
                  decoration: BoxDecoration(
                    color: ItemKind.note.tint(brightness),
                    borderRadius: BorderRadius.circular(9),
                  ),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 8,
                      vertical: 2,
                    ),
                    child: Text(
                      '${health.length} to fix',
                      style: TextStyle(
                        color: ItemKind.note.color(brightness),
                        fontSize: 13,
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                  ),
                ),
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
              const JewelTile(jewel: Jewel.allAccounts, size: 40),
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
/// [action]: its label, and what it does. [inPlace] puts it where the one
/// showing is, without waiting that one out.
void showMessage(
  BuildContext context,
  String text, {
  (String, VoidCallback)? action,
  int? seconds,
  bool inPlace = false,
}) {
  final messenger = ScaffoldMessenger.of(context);
  if (inPlace) {
    messenger
      ..clearSnackBars()
      ..removeCurrentSnackBar();
  }
  messenger.showSnackBar(
    SnackBar(
      content: Text(text),
      duration: Duration(seconds: seconds ?? (action == null ? 3 : 6)),
      action: action == null
          ? null
          : SnackBarAction(label: action.$1, onPressed: action.$2),
    ),
  );
}

/// The kinds of item on a phone, each a chip with its jewel tile and how
/// many there are, in one row that scrolls sideways: every name on one line,
/// at any text size, so the favorites under them stay in sight. Only kinds
/// that have items, as the web vault lists them.
class CategoryTiles extends StatelessWidget {
  const CategoryTiles({
    super.key,
    required this.counts,
    required this.onChoose,
  });

  final Map<String, int> counts;
  final ValueChanged<String> onChoose;

  @override
  Widget build(BuildContext context) {
    final kinds = ItemKind.all
        .where((k) => (counts[k.kind] ?? 0) > 0)
        .toList();
    if (kinds.isEmpty) return const SizedBox.shrink();
    return SingleChildScrollView(
      key: const ValueKey('kinds'),
      scrollDirection: Axis.horizontal,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      child: Row(
        children: [
          for (final (i, kind) in kinds.indexed) ...[
            if (i > 0) const SizedBox(width: 8),
            _chip(context, kind),
          ],
        ],
      ),
    );
  }

  Widget _chip(BuildContext context, ItemKind kind) => RestingCard(
    radius: 14,
    child: InkWell(
      onTap: () => onChoose(kind.kind),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 52),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(8, 8, 14, 8),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              JewelTile(jewel: kind, size: 34),
              const SizedBox(width: 10),
              Text(
                kind.many,
                maxLines: 1,
                style: const TextStyle(
                  fontSize: 15,
                  fontWeight: FontWeight.w600,
                ),
              ),
              const SizedBox(width: 8),
              Text(
                '${counts[kind.kind] ?? 0}',
                style: TextStyle(fontSize: 14, color: context.colors.muted),
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
    this.onLongPress,
    this.checked,
    this.dimmed = false,
    this.badge,
  });

  final Map<String, dynamic> item;
  final BorderRadius radius;
  final VoidCallback? onTap;

  /// Starts selecting, with this one.
  final VoidCallback? onLongPress;

  /// While items are being selected, whether this one is: its checkbox is
  /// where its tile was, and a tap selects it instead of opening it.
  final bool? checked;

  /// One that can't be selected while others are.
  final bool dimmed;

  /// Its account's, where several accounts' items are listed together.
  final Widget? badge;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final subtitle = item['subtitle'] as String;
    final selecting = checked != null;
    final row = InkWell(
      borderRadius: radius,
      onTap: onTap,
      onLongPress: onLongPress,
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 64),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 10, 12, 10),
          child: Row(
            children: [
              selecting
                  ? SelectBox(checked: checked!, name: item['name'] as String)
                  : Avatar.of(item, size: 40),
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
              // It doesn't open while selecting.
              Opacity(
                opacity: selecting || dimmed ? 0 : 1,
                child: Glyph(
                  Glyphs.chevron,
                  size: 18,
                  color: c.muted,
                  strokeWidth: 2,
                ),
              ),
            ],
          ),
        ),
      ),
    );
    if (dimmed) return Opacity(opacity: 0.45, child: row);
    if (!selecting) return row;
    return Semantics(
      selected: checked,
      child: checked!
          ? DecoratedBox(
              decoration: BoxDecoration(color: c.tint, borderRadius: radius),
              child: row,
            )
          : row,
    );
  }
}

/// Whether an item in a list is the account shown's: "All accounts" lists
/// the others' too, which open in their own account.
bool ofThisAccount(Map<String, dynamic> item) {
  final account = item['account'] as String?;
  return account == null || account == Core.account;
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

/// The menu on a computer, as the web vault has it: a card on the ground
/// with All items and Favorites, the kinds of item that have items, with
/// their jewel tiles, and the folders (Personal and its folders, then each
/// organization and its shared folders, nested), each with how many it
/// holds; then who's signed in, the theme, Settings, and Lock.
class Sidebar extends StatelessWidget {
  const Sidebar({
    super.key,
    this.width = sidebarWidth,
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
    this.onSend,
    this.onBrowsers,
    this.onImport,
    this.onExport,
    this.archived = 0,
    this.trashed = 0,
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

  /// The Tools: the generator, and importing and exporting a file
  /// (transfer.dart).
  final VoidCallback? onGenerator;

  /// Send (send.dart), in a window of its own.
  final VoidCallback? onSend;

  /// Importing from this computer's browsers (browser_import.dart).
  final VoidCallback? onBrowsers;
  final VoidCallback? onImport;
  final VoidCallback? onExport;

  /// How many items are in Archive, and in the trash beside it.
  final int archived;
  final int trashed;
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
    Widget entry({
      required Widget tile,
      required String label,
      required bool on,
      required VoidCallback tap,
      int? count,
      Widget? trailing,
      int depth = 0,
      bool strong = false,
      Widget? note,
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
                  if (note case final pill?)
                    pill
                  else if (count != null)
                    Text(
                      '$count',
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
    // Health's count of items to fix, as the web vault has it: a topaz pill,
    // hidden at none.
    final brightness = Theme.of(context).brightness;
    final toFixPill = toFix == 0
        ? null
        : Semantics(
            label: '$toFix to fix',
            child: ExcludeSemantics(
              child: DecoratedBox(
                key: const ValueKey('health-count'),
                decoration: BoxDecoration(
                  color: ItemKind.note.tint(brightness),
                  borderRadius: BorderRadius.circular(9),
                ),
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
                  child: Text(
                    '$toFix',
                    style: TextStyle(
                      color: ItemKind.note.color(brightness),
                      fontSize: 12,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ),
              ),
            ),
          );
    // The tools, after a line under the folders and the trash, as the web
    // vault's menu has them: the generator and Send, then importing (a
    // file, and this computer's browsers), then exporting.
    // The list fades at its foot, so whatever is past it reads as more.
    final tools = [
      if (onGenerator case final open?)
        entry(
          tile: outline(Glyphs.generator),
          label: 'Generator',
          on: false,
          tap: open,
        ),
      if (onSend case final open?)
        entry(
          tile: outline(Glyphs.send),
          label: 'Send',
          on: false,
          tap: open,
        ),
      if (onImport case final open?)
        entry(
          tile: outline(Glyphs.download),
          label: 'Import',
          on: false,
          tap: open,
        ),
      if (onBrowsers case final open?)
        entry(
          tile: outline(Glyphs.download),
          label: 'Import from browsers',
          on: false,
          tap: open,
        ),
      if (onExport case final open?)
        entry(
          tile: outline(Glyphs.upload),
          label: 'Export',
          on: false,
          tap: open,
        ),
    ];
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
            // The list fades out at its foot rather than cutting an entry in
            // half, as the web vault's menu does; scrolled to its end, the
            // last entry sits above the fade.
            child: ShaderMask(
              blendMode: BlendMode.dstIn,
              shaderCallback: (bounds) => LinearGradient(
                begin: Alignment.topCenter,
                end: Alignment.bottomCenter,
                colors: const [Colors.black, Colors.black, Colors.transparent],
                stops: [
                  0,
                  bounds.height <= 28 ? 0 : 1 - 28 / bounds.height,
                  1,
                ],
              ).createShader(bounds),
              child: ListView(
              padding: const EdgeInsets.fromLTRB(8, 0, 8, 28),
              children: [
                // Every unlocked account's items together, with more than
                // one account signed in; "All items" are this one's.
                if (menu?.several ?? false)
                  entry(
                    tile: const JewelTile(jewel: Jewel.allAccounts, size: 24),
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
                // Once something is a favorite (or while it's shown).
                if (count(Filter.favorites) > 0 || filter == Filter.favorites)
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
                  note: toFixPill,
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
                // Archive, then the trash, after the folders as there.
                entry(
                  tile: outline(Glyphs.archive, strong: filter == Filter.archive),
                  label: 'Archive',
                  on: filter == Filter.archive,
                  tap: () => onChoose(Filter.archive),
                  count: archived,
                ),
                entry(
                  tile: outline(Glyphs.trash, strong: filter == Filter.trash),
                  label: 'Trash',
                  on: filter == Filter.trash,
                  tap: () => onChoose(Filter.trash),
                  count: trashed,
                ),
                if (tools.isNotEmpty) ...[
                  const Padding(
                    padding: EdgeInsets.symmetric(horizontal: 6, vertical: 10),
                    child: Divider(),
                  ),
                  ...tools,
                ],
              ],
            ),
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
/// and how many, and the items, the chosen one raised. Several can be
/// selected (selection.dart): each row has a checkbox where its tile is,
/// under the pointer and whenever any is selected; Shift and a click takes
/// a range, Command (Ctrl off a Mac) and a click one without opening it,
/// and with the list in focus Command or Ctrl with A takes them all and
/// Escape none. Select, beside the count, shows every row's checkbox and
/// makes a click select. While any is selected, or Select is on,
/// [selection] is where the title was.
class ItemList extends StatefulWidget {
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
    this.emptyState,
    this.header,
    this.checked = const {},
    this.selectMode = false,
    this.onSelectMode,
    this.onCheck,
    this.onCheckTo,
    this.onCheckAll,
    this.onUncheckAll,
    this.selection,
  });

  /// What it says with nothing to show: why, by what it shows.
  final String empty;

  /// What it shows in place of [empty] when the vault has nothing in it yet:
  /// what to do first ([EmptyVault]).
  final Widget? emptyState;

  /// Over the rows, under the title: Health's reports, and who sees how many
  /// of the person's own passwords are weak, reused, or exposed.
  final Widget? header;

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

  /// The items selected to do one thing to all of them, by key
  /// (`selectionKey`).
  final Set<String> checked;

  /// Select is on: every row's checkbox shows, and a click selects.
  final bool selectMode;

  /// Turns Select on (its button beside the count).
  final VoidCallback? onSelectMode;

  /// Selects an item or takes it out; without it nothing is selected here.
  final ValueChanged<Map<String, dynamic>>? onCheck;

  /// Selects up to an item, from the one selected last.
  final ValueChanged<Map<String, dynamic>>? onCheckTo;
  final VoidCallback? onCheckAll;
  final VoidCallback? onUncheckAll;

  /// How many are selected and what to do with them, in the title's place.
  final Widget? selection;

  @override
  State<ItemList> createState() => _ItemListState();
}

class _ItemListState extends State<ItemList> {
  /// The list's own focus, which a click on a row takes: the keys that
  /// select are its, not the search field's.
  final focus = FocusNode(debugLabel: 'items', skipTraversal: true);

  @override
  void dispose() {
    focus.dispose();
    super.dispose();
  }

  KeyEventResult key(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent || widget.onCheck == null) {
      return KeyEventResult.ignored;
    }
    if (event.logicalKey == LogicalKeyboardKey.keyA && commandHeld()) {
      widget.onCheckAll?.call();
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.escape &&
        (widget.checked.isNotEmpty || widget.selectMode)) {
      widget.onUncheckAll?.call();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  /// A click on a row: with Shift, a range; with Command or Ctrl, or while
  /// Select is on, this one without opening it; else it opens.
  void tap(Map<String, dynamic> item) {
    focus.requestFocus();
    if (widget.onCheck case final toggle?) {
      if (HardwareKeyboard.instance.isShiftPressed) {
        widget.onCheckTo?.call(item);
        return;
      }
      if (commandHeld() || widget.selectMode) {
        toggle(item);
        return;
      }
    }
    widget.onSelect(item);
  }

  /// Select, beside the count: the list takes the focus, so Escape and
  /// Command or Ctrl with A are its at once.
  void startSelecting() {
    focus.requestFocus();
    widget.onSelectMode?.call();
  }

  /// A click on a row's checkbox.
  void check(Map<String, dynamic> item) {
    focus.requestFocus();
    if (HardwareKeyboard.instance.isShiftPressed) {
      widget.onCheckTo?.call(item);
    } else {
      widget.onCheck?.call(item);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final items = widget.items;
    final selection = widget.selection;
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
                  onChanged: widget.onSearch,
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
              widget.syncing
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
                      onPressed: widget.onSync,
                    ),
              const SizedBox(width: 4),
              Tooltip(
                message: 'New item',
                excludeFromSemantics: true,
                child: FilledButton.icon(
                  onPressed: widget.onNew,
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
        if (widget.banner != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 12, 0),
            child: ClipRRect(
              borderRadius: BorderRadius.circular(12),
              child: widget.banner,
            ),
          ),
        Expanded(
          child: Focus(
            focusNode: focus,
            onKeyEvent: key,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                // The title keeps its room while the selection's bar is in
                // its place, so the rows stay where they are.
                Stack(
                  children: [
                    Visibility(
                      visible: selection == null,
                      maintainSize: true,
                      maintainAnimation: true,
                      maintainState: true,
                      child: Padding(
                        padding: const EdgeInsets.fromLTRB(22, 10, 18, 6),
                        child: Row(
                          crossAxisAlignment: CrossAxisAlignment.baseline,
                          textBaseline: TextBaseline.alphabetic,
                          children: [
                            Expanded(
                              child: Text(
                                widget.title,
                                overflow: TextOverflow.ellipsis,
                                style: const TextStyle(
                                  fontSize: 22,
                                  fontWeight: FontWeight.w700,
                                ),
                              ),
                            ),
                            Text(
                              items.length == 1
                                  ? '1 item'
                                  : '${items.length} items',
                              style: TextStyle(color: c.muted, fontSize: 13),
                            ),
                            if (widget.onSelectMode != null &&
                                widget.onCheck != null &&
                                items.isNotEmpty) ...[
                              const SizedBox(width: 6),
                              TextButton(
                                onPressed: startSelecting,
                                style: TextButton.styleFrom(
                                  minimumSize: const Size(0, 30),
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: 10,
                                  ),
                                  tapTargetSize:
                                      MaterialTapTargetSize.shrinkWrap,
                                  visualDensity: VisualDensity.compact,
                                ),
                                child: const Text('Select'),
                              ),
                            ],
                          ],
                        ),
                      ),
                    ),
                    if (selection != null) Positioned.fill(child: selection),
                  ],
                ),
                if (widget.header case final header?)
                  Padding(
                    padding: const EdgeInsets.fromLTRB(20, 2, 18, 10),
                    child: header,
                  ),
                Expanded(
                  child: items.isEmpty
                      ? widget.emptyState ??
                            Center(
                              child: Padding(
                                padding: const EdgeInsets.symmetric(
                                  horizontal: 24,
                                ),
                                child: Text(
                                  widget.empty,
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
                            return ItemRow(
                              key: ValueKey((account, item['id'])),
                              item: item,
                              on:
                                  item['id'] == widget.selected &&
                                  ofThisAccount(item),
                              where: widget.badges
                                  ? item['where'] as String?
                                  : null,
                              account: widget.accounts?.entry(account),
                              checked: widget.checked.contains(
                                selectionKey(item),
                              ),
                              selecting:
                                  widget.checked.isNotEmpty ||
                                  widget.selectMode,
                              onTap: () => tap(item),
                              onCheck: widget.onCheck != null
                                  ? () => check(item)
                                  : null,
                            );
                          },
                        ),
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

/// Health's reports, each with its tile, as the web vault's Reports page
/// draws them (weak ruby, reused topaz, unsecured websites aquamarine,
/// expiring items onyx): its key (`inReport`), its name, and its jewel.
const healthReports = [
  (
    'weak',
    'Weak passwords',
    Jewel(Glyphs.key, Color(0xFFB01E54), Color(0xFFFF92B8)),
  ),
  (
    'reused',
    'Reused passwords',
    Jewel(Glyphs.copy, Color(0xFFA9600C), Color(0xFFF2B766)),
  ),
  (
    'unsecured',
    'Unsecured websites',
    Jewel(Glyphs.globe, Color(0xFF1C7FB8), Color(0xFF7CC4EF)),
  ),
  ('expiring', 'Expiring items', Jewel.expiring),
];

/// Health's summary: each report with something in it, with how many
/// items, as choices over the list. Picking one shows only its items;
/// picking it again shows everything to fix.
class HealthReports extends StatelessWidget {
  const HealthReports({
    super.key,
    required this.health,
    required this.report,
    required this.onReport,
  });

  /// What each item has to fix, by its id.
  final Map<String, Map<String, dynamic>> health;
  final String? report;
  final ValueChanged<String?> onReport;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final counts = {
      for (final (key, _, _) in healthReports)
        key: health.values.where((i) => inReport(i, key)).length,
    };
    final reports = [
      for (final r in healthReports)
        if (counts[r.$1]! > 0 || r.$1 == report) r,
    ];
    if (reports.isEmpty) return const SizedBox.shrink();
    return Wrap(
      key: const ValueKey('health-reports'),
      spacing: 8,
      runSpacing: 8,
      children: [
        for (final (key, label, jewel) in reports)
          Semantics(
            button: true,
            selected: report == key,
            child: Material(
              color: report == key ? c.tint : c.pane,
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(12),
                side: BorderSide(
                  color: report == key ? c.selectedRing : c.line,
                ),
              ),
              child: InkWell(
                customBorder: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(12),
                ),
                onTap: () => onReport(report == key ? null : key),
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(8, 7, 12, 7),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      JewelTile(jewel: jewel, size: 28),
                      const SizedBox(width: 8),
                      Text(
                        label,
                        style: const TextStyle(
                          fontSize: 14,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      const SizedBox(width: 8),
                      Text(
                        '${counts[key]}',
                        style: TextStyle(color: c.muted, fontSize: 13),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
      ],
    );
  }
}

/// A vault with nothing in it yet, on a computer or a phone: what to do
/// first, with New and the ways to bring passwords in, in place of a list.
class EmptyVault extends StatelessWidget {
  const EmptyVault({
    super.key,
    required this.onNew,
    this.onBrowsers,
    this.onImport,
  });

  final VoidCallback onNew;

  /// This computer's browsers (browser_import.dart).
  final VoidCallback? onBrowsers;

  /// Importing a file (transfer.dart): an export from another password
  /// manager or a browser.
  final VoidCallback? onImport;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Center(
      child: SingleChildScrollView(
        padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 24),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 360),
          child: Column(
            key: const ValueKey('empty-vault'),
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const Center(child: JewelTile(jewel: Jewel.allItems, size: 56)),
              const SizedBox(height: 16),
              const Text(
                'Nothing saved yet',
                textAlign: TextAlign.center,
                style: TextStyle(fontSize: 20, fontWeight: FontWeight.w700),
              ),
              const SizedBox(height: 6),
              Text(
                'Add your first item, or bring in the passwords you have now.',
                textAlign: TextAlign.center,
                style: TextStyle(color: c.muted, height: 1.35),
              ),
              const SizedBox(height: 18),
              FilledButton.icon(
                onPressed: onNew,
                icon: Glyph(
                  Glyphs.plus,
                  size: 18,
                  strokeWidth: 2.4,
                  color: Theme.of(context).colorScheme.onPrimary,
                ),
                label: const Text('New item'),
              ),
              if (onBrowsers case final browsers?) ...[
                const SizedBox(height: 8),
                OutlinedButton.icon(
                  onPressed: browsers,
                  icon: Glyph(Glyphs.download, size: 18, color: c.muted),
                  label: const Text('Import from browsers'),
                ),
              ],
              if (onImport case final fromFile?) ...[
                const SizedBox(height: 8),
                OutlinedButton.icon(
                  onPressed: fromFile,
                  icon: Glyph(Glyphs.download, size: 18, color: c.muted),
                  label: const Text('Import a file'),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// An item's row in a computer's list. Its checkbox is where its tile is
/// while the pointer is over it or any item is selected; on a touch screen a
/// long press selects it.
class ItemRow extends StatefulWidget {
  const ItemRow({
    super.key,
    required this.item,
    required this.on,
    required this.checked,
    required this.selecting,
    required this.onTap,
    this.onCheck,
    this.where,
    this.account,
  });

  final Map<String, dynamic> item;

  /// It's the one open beside the list.
  final bool on;

  /// It's selected, and whether any item is.
  final bool checked;
  final bool selecting;
  final VoidCallback onTap;

  /// Selects it or takes it out; null where nothing is selected.
  final VoidCallback? onCheck;

  /// Where it is, beside it.
  final String? where;

  /// Its account, beside it, in "All accounts".
  final AccountEntry? account;

  @override
  State<ItemRow> createState() => _ItemRowState();
}

class _ItemRowState extends State<ItemRow> {
  bool hovered = false;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final item = widget.item;
    final subtitle = item['subtitle'] as String;
    final box = widget.onCheck != null && (hovered || widget.selecting);
    return MouseRegion(
      onEnter: (_) => setState(() => hovered = true),
      onExit: (_) => setState(() => hovered = false),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 2),
        // The chosen one rises off the ground, in the selection's ring.
        child: DecoratedBox(
          decoration: widget.on
              ? c.lifted.decoration(
                  BorderRadius.circular(12),
                  ring: c.selectedRing,
                )
              : const BoxDecoration(),
          child: Material(
            color: widget.checked ? c.tint : Colors.transparent,
            borderRadius: BorderRadius.circular(12),
            child: InkWell(
              borderRadius: BorderRadius.circular(12),
              onTap: widget.onTap,
              onLongPress: widget.onCheck,
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 10,
                ),
                child: Row(
                  children: [
                    box
                        ? SelectBox(
                            checked: widget.checked,
                            name: item['name'] as String,
                            onChanged: widget.onCheck,
                          )
                        : Avatar.of(item, size: 40),
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
                              fontSize: 15,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                          if (subtitle.isNotEmpty)
                            Text(
                              subtitle,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(color: c.muted, fontSize: 13),
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
                    if (widget.where case final where?) WhereBadge(where),
                    if (widget.account case final account?)
                      AccountBadge(entry: account),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
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
    this.onArchive,
    this.onRestore,
    this.onDeleteForever,
    this.onChanged,
    this.server,
    this.phone = false,
    this.atRisk = false,
  });

  /// Health has something to fix with its password (a weak or reused one,
  /// or a website without https): "Change password" opens the site's page.
  final bool atRisk;

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

  /// Archives it (true) or takes it out of the archive, by its name.
  final void Function(String name, bool archive)? onArchive;

  /// In the trash: restores it, or deletes it for good, by its name.
  final ValueChanged<String>? onRestore;
  final ValueChanged<String>? onDeleteForever;

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
    uploadingHere = uploading.value?.isFor(widget.id) ?? false;
    uploading.addListener(uploadChanged);
  }

  void tick() {
    if (busy.value || item?['hasTotp'] != true) return;
    setState(() => totp = widget.core.totp(widget.id));
  }

  @override
  void dispose() {
    uploading.removeListener(uploadChanged);
    ticker?.cancel();
    masterPassword.dispose();
    super.dispose();
  }

  /// Whether a file is on its way to this item (attaching.dart).
  bool uploadingHere = false;

  /// Shows how far the upload is; once one for this item ends, the item as
  /// it is now (the file, if the server confirmed it).
  void uploadChanged() {
    if (!mounted) return;
    final here = uploading.value?.isFor(widget.id) ?? false;
    setState(() {
      if (uploadingHere && !here && !busy.value) {
        item = widget.core.item(widget.id);
      }
      uploadingHere = here;
    });
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

  /// Opening the site's change-password page: its button spins meanwhile.
  bool changing = false;

  /// "Change password", as the web vault has it on an at-risk login: the
  /// site's own change-password page, in the browser (the core finds it:
  /// core/src/change_password.rs).
  Future<void> changePassword() async {
    if (changing) return;
    setState(() => changing = true);
    final answer = await Core.changePasswordPage(widget.id);
    if (!mounted) return;
    setState(() => changing = false);
    final page = answer.body['url'] as String?;
    if (!answer.ok || page == null) {
      showMessage(context, answer.error ?? 'Couldn\'t find the site\'s page.');
      return;
    }
    await openSite(page);
  }

  /// Whether the person may do something with the item (`edit`,
  /// `seePasswords`, `delete`, ...), as their shared folders allow (the
  /// core says yes for their own items); no when the core doesn't say.
  bool may(Map<String, dynamic> item, String what) =>
      (item['permissions'] as Map?)?[what] == true;

  /// The attachments being downloaded and saved: each one's button spins
  /// until it's done.
  final saving = <String>{};

  /// Downloads, opens, and saves an attachment to Downloads, through the core.
  Future<void> saveAttachment(String attachmentId, String name) async {
    if (saving.contains(attachmentId)) return;
    if (asksAgain()) {
      setState(() {});
      return;
    }
    setState(() => saving.add(attachmentId));
    final Answer answer;
    try {
      answer = await whileBusy(
        () => Core.saveAttachment(widget.id, attachmentId, name),
      );
    } finally {
      if (mounted) setState(() => saving.remove(attachmentId));
    }
    if (!mounted) return;
    showMessage(
      context,
      answer.ok
          ? 'Saved to ${answer.body['path']}'
          : answer.error ?? 'Couldn\'t save that.',
    );
  }

  /// Asks for a file with the device's picker and attaches it, with how far
  /// it is shown on the item (attaching.dart). Only the server's say makes
  /// it attached: a file it didn't confirm isn't listed.
  Future<void> attach() async {
    if (asksAgain()) {
      setState(() {});
      return;
    }
    if (uploading.value != null) {
      showMessage(context, anotherUploading);
      return;
    }
    final limit = widget.core.attachLimit();
    Map<String, dynamic>? picked;
    try {
      picked = await Core.pickFile((limit['bytes'] as num? ?? 0).toInt());
    } on PlatformException catch (e) {
      picked = {'error': e.message};
    } on MissingPluginException {
      picked = {'error': null};
    }
    if (picked == null) return;
    final path = picked['path'] as String?;
    if (!mounted || path == null) {
      await Core.forgetPicked();
      if (!mounted) return;
      showMessage(
        context,
        picked['tooLarge'] == true
            ? limit['refusal'] as String? ?? 'That file is too large.'
            : picked['error'] as String? ??
                  'PassBundle couldn\'t open that file.',
      );
      return;
    }
    final name = picked['name'] as String? ?? '';
    // Said wherever the person is by the time it ends; the file's name only
    // while its item is still shown.
    final messenger = ScaffoldMessenger.of(context);
    final answer = await attachFile(widget.core, widget.id, path, name);
    if (answer.body['canceled'] == true) return;
    final said = answer.ok
        ? (answer.body['synced'] == false
              ? 'The server has ${mounted ? name : 'the file'}. It shows here once PassBundle syncs.'
              : 'Attached ${mounted ? name : 'the file'}.')
        : answer.error ?? 'Couldn\'t attach that file.';
    messenger.showSnackBar(
      SnackBar(content: Text(said), duration: const Duration(seconds: 4)),
    );
    if (!mounted) return;
    setState(() => item = widget.core.item(widget.id));
    widget.onChanged?.call();
  }

  /// Deletes one of its attachments from the server, once the person says so.
  Future<void> deleteAttachment(String attachmentId, String name) async {
    if (asksAgain()) {
      setState(() {});
      return;
    }
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Delete $name?'),
        content: const Text(
          'It\'s deleted from the server, for every device. It can\'t be restored.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            style: dangerButton(context),
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Delete'),
          ),
        ],
      ),
    );
    if (sure != true || !mounted) return;
    final answer = await whileBusy(
      () => Core.deleteAttachment(widget.id, attachmentId),
    );
    if (!mounted) return;
    setState(() => item = widget.core.item(widget.id));
    widget.onChanged?.call();
    showMessage(
      context,
      answer.ok
          ? (answer.body['synced'] == false
                ? 'Deleted $name. PassBundle couldn\'t sync the rest just now, and will next time.'
                : 'Deleted $name.')
          : answer.error ?? 'Couldn\'t delete that.',
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

  /// What can be done with it: favorite it, edit it, archive it, and move
  /// it to the trash; or, in the trash, restore it and delete it for good.
  List<Widget> actions(Map<String, dynamic> item) {
    if (item['deleted'] == true) return trashActions(item);
    return ownActions(item);
  }

  /// In the trash: Restore and Delete forever, where the person may delete
  /// it (the server's check for both). Nothing else changes an item there.
  List<Widget> trashActions(Map<String, dynamic> item) => [
    if (widget.onRestore case final restore? when may(item, 'delete'))
      IconButton(
        tooltip: 'Restore',
        icon: const Icon(Icons.restore_from_trash_rounded),
        onPressed: () => restore(item['name'] as String),
      ),
    if (widget.onDeleteForever case final delete? when may(item, 'delete'))
      IconButton(
        tooltip: 'Delete forever',
        icon: const Icon(Icons.delete_forever_rounded),
        onPressed: () => delete(item['name'] as String),
      ),
  ];

  /// Out of the trash: favorite it, edit it, archive it, and move it to the
  /// trash.
  List<Widget> ownActions(Map<String, dynamic> item) => [
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
    // A phone's top bar has room for icons only, and an unlabeled one would
    // look like Open website: there it's a labeled button under the item's
    // name ([webVaultButton]).
    else if (item['editInWebVault'] == true &&
        widget.server != null &&
        !widget.phone)
      Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4),
        child: webVaultButton(),
      ),
    // The person's own mark, as a favorite is: any item they see takes it.
    if (widget.onArchive case final archive?)
      IconButton(
        key: const ValueKey('archive-item'),
        tooltip: item['archived'] == true ? 'Move out of Archive' : 'Archive',
        icon: Glyph(
          Glyphs.archive,
          size: 22,
          color: Theme.of(context).colorScheme.onSurfaceVariant,
        ),
        onPressed: () =>
            archive(item['name'] as String, item['archived'] != true),
      ),
    if (may(item, 'delete'))
      IconButton(
        tooltip: 'Move to trash',
        icon: const Icon(Icons.delete_outline_rounded),
        onPressed: () => widget.onTrash(item['name'] as String),
      ),
  ];

  /// Edit in web vault, for a kind the app doesn't edit yet, saying so.
  Widget webVaultButton() => Tooltip(
    message: 'PassBundle edits this kind in the web vault, for now',
    child: OutlinedButton.icon(
      key: const ValueKey('edit-in-web-vault'),
      onPressed: editInWebVault,
      icon: const Icon(Icons.open_in_new_rounded, size: 16),
      label: const Text('Edit in web vault'),
    ),
  );

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
    // Notes are plain to the person's shared folder: shown and copied by
    // anyone who can open the item.
    Widget secretRow(String label, String what) => row(
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
        if (sees || what == 'notes') copyButton(label.toLowerCase(), what),
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
    if (item['codeNeedsPremium'] == true) {
      // A key whose code needs Premium (core/src/codes.rs): no code, no
      // Copy, and the editor still has the key.
      rows.add(
        row(
          'One-time code',
          Text(
            'Two-step login codes are part of Premium.',
            key: const ValueKey('code-needs-premium'),
            style: TextStyle(fontSize: valueSize),
          ),
        ),
      );
    } else if (item['hasTotp'] == true && !sees) {
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
    // Attachments: saved to Downloads, or to Files on an iPhone; deleted
    // where the person may edit the item.
    final changesAttachments = item['changesAttachments'] == true;
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
            if (changesAttachments)
              IconButton(
                tooltip: 'Delete ${a['name']}',
                icon: Icon(Icons.delete_outline_rounded, color: c.muted),
                onPressed: () =>
                    deleteAttachment(a['id'] as String, a['name'] as String),
              ),
            if (a['opens'] != false)
              action(
                'Save ${a['name']}',
                saving.contains(a['id'])
                    ? SizedBox.square(
                        dimension: 20,
                        child: CircularProgressIndicator(
                          strokeWidth: 2,
                          semanticsLabel: 'Saving ${a['name']}',
                        ),
                      )
                    : Icon(
                        Icons.download_rounded,
                        size: 20,
                        color: scheme.primary,
                      ),
                () => saveAttachment(a['id'] as String, a['name'] as String),
              ),
          ],
        ),
      );
    }
    // The file on its way here, with how far it is and Cancel.
    final upload = uploading.value;
    if (upload != null && upload.isFor(widget.id)) {
      rows.add(
        row(
          'Attaching',
          Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                upload.name,
                style: TextStyle(fontSize: valueSize),
                overflow: TextOverflow.ellipsis,
              ),
              const SizedBox(height: 8),
              LinearProgressIndicator(
                key: const ValueKey('upload-progress'),
                value: upload.canceling ? null : upload.fraction,
                borderRadius: BorderRadius.circular(2),
              ),
              const SizedBox(height: 6),
              Text(
                uploadWords(upload),
                style: TextStyle(color: c.muted, fontSize: 13),
              ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: upload.canceling
                  ? null
                  : () => cancelUpload(widget.core),
              child: const Text('Cancel'),
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
      rows.add(secretRow('Notes', 'notes'));
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
        : Categories.forItem(
            item['category'] as String,
            item['templateId'] as String?,
          );
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
    // One of the person's own, where an organization keeps items out of
    // Personal: the server refuses a change to it, so it has no Edit, and
    // says why (core/src/ffi.rs, pb_item's `keptOutBy`).
    if (item['keptOutBy'] case final String org) {
      rows.add(
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
          child: Text(
            '$org keeps items out of Personal, so this item can\'t be changed. You can move it to a shared folder in the web vault.',
            key: const ValueKey('kept-out'),
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
      // A login with something to fix: the site's change-password page.
      if (widget.atRisk && item['kind'] == 'login' && may(item, 'edit'))
        Padding(
          padding: const EdgeInsets.only(top: 10),
          child: Align(
            alignment: Alignment.centerLeft,
            child: TextButton.icon(
              key: const ValueKey('change-password'),
              onPressed: changing ? null : changePassword,
              icon: changing
                  ? const SizedBox.square(
                      dimension: 16,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : Icon(
                      Icons.open_in_new_rounded,
                      size: 18,
                      color: scheme.primary,
                    ),
              label: const Text('Change password'),
            ),
          ),
        ),
      if (changesAttachments && !(upload?.isFor(widget.id) ?? false))
        Padding(
          padding: const EdgeInsets.only(top: 10),
          child: Align(
            alignment: Alignment.centerLeft,
            child: TextButton.icon(
              key: const ValueKey('attach-file'),
              onPressed: attach,
              icon: Icon(
                Icons.attach_file_rounded,
                size: 18,
                color: scheme.primary,
              ),
              label: const Text('Attach a file'),
            ),
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
          if (item['editInWebVault'] == true &&
              item['editable'] == false &&
              widget.server != null) ...[
            const SizedBox(height: 14),
            Center(child: webVaultButton()),
          ],
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
                              '$what in $where',
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

/// What an upload is doing, in words (attaching.dart).
String uploadWords(Uploading upload) {
  if (upload.canceling) return 'Canceling';
  return switch (upload.phase) {
    'reading' => 'Reading the file',
    'encrypting' => 'Encrypting it on this device',
    'sending' when upload.total > 0 =>
      'Sent ${fileSize(upload.done)} of ${fileSize(upload.total)}',
    'sending' => 'Sending',
    'finishing' => 'Checking that the server has it',
    _ => 'Starting',
  };
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

/// A new folder, as [askNewFolder] gives it: its whole name (a nested one
/// after its parent's, "Work/Clients", as upstream's apps nest them), and
/// the organization it's in, or null for Personal.
typedef NewFolder = ({String name, String? organization});

/// Asks for a new folder's name and where it goes: Personal or an
/// organization where the person may make shared folders (`canMakeFolders`),
/// at the top or inside one of its folders. Null when canceled.
Future<NewFolder?> askNewFolder(BuildContext context, Places places) =>
    showDialog<NewFolder>(
      context: context,
      builder: (context) => _NewFolderDialog(places: places),
    );

class _NewFolderDialog extends StatefulWidget {
  const _NewFolderDialog({required this.places});

  final Places places;

  @override
  State<_NewFolderDialog> createState() => _NewFolderDialogState();
}

class _NewFolderDialogState extends State<_NewFolderDialog> {
  final field = TextEditingController();

  /// Where it can go, in the menu's order: (what the list says, how deep,
  /// whether it's an owner, the organization, the parent's whole name).
  late final List<(String, int, bool, String?, String?)> places = [
    ('Personal', 0, true, null, null),
    for (final f in widget.places.folders)
      (
        f['label'] as String,
        1 + (f['depth'] as int),
        false,
        null,
        f['name'] as String,
      ),
    for (final o in widget.places.organizations)
      if (o['canMakeFolders'] == true) ...[
        (o['name'] as String, 0, true, o['id'] as String, null),
        for (final c in (o['collections'] as List).cast<Map<String, dynamic>>())
          (
            c['label'] as String,
            1 + (c['depth'] as int),
            false,
            o['id'] as String,
            c['name'] as String,
          ),
      ],
  ];
  int at = 0;

  @override
  void dispose() {
    field.dispose();
    super.dispose();
  }

  void done() {
    final name = field.text.trim();
    if (name.isEmpty) return;
    final (_, _, _, organization, parent) = places[at];
    Navigator.pop<NewFolder>(context, (
      name: parent == null ? name : '$parent/$name',
      organization: organization,
    ));
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return AlertDialog(
      title: const Text('New folder'),
      content: SizedBox(
        width: 380,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            TextField(
              controller: field,
              autofocus: true,
              decoration: const InputDecoration(labelText: 'Name'),
              onChanged: (_) => setState(() {}),
              onSubmitted: (_) => done(),
            ),
            // Where it goes, when there's more than Personal's top.
            if (places.length > 1) ...[
              const SizedBox(height: 16),
              DropdownButtonFormField<int>(
                key: const ValueKey('new-folder-in'),
                initialValue: at,
                isExpanded: true,
                decoration: const InputDecoration(labelText: 'In'),
                selectedItemBuilder: (_) => [
                  for (final (label, _, _, _, _) in places)
                    Align(
                      alignment: AlignmentDirectional.centerStart,
                      child: Text(label, overflow: TextOverflow.ellipsis),
                    ),
                ],
                items: [
                  for (final (i, (label, depth, owner, _, _)) in places.indexed)
                    DropdownMenuItem(
                      value: i,
                      child: Padding(
                        padding: EdgeInsets.only(left: 18.0 * depth),
                        child: Row(
                          children: [
                            Glyph(
                              owner
                                  ? (i == 0 ? Glyphs.person : Glyphs.building)
                                  : places[i].$4 == null
                                  ? Glyphs.folder
                                  : Glyphs.sharedFolder,
                              size: 18,
                              color: owner
                                  ? Theme.of(context).colorScheme.onSurface
                                  : c.muted,
                            ),
                            const SizedBox(width: 10),
                            Expanded(
                              child: Text(
                                label,
                                overflow: TextOverflow.ellipsis,
                                style: owner
                                    ? const TextStyle(fontWeight: FontWeight.w700)
                                    : null,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                ],
                onChanged: (i) => setState(() => at = i ?? 0),
              ),
              if (places[at].$4 != null)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(
                    'Owners and admins see it, and give others access in the web vault.',
                    style: TextStyle(color: c.muted, fontSize: 13),
                  ),
                ),
            ],
          ],
        ),
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
}

/// A folder's name, asked for: null when canceled.
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
