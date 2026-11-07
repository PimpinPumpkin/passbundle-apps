// PassBundle: sign in to a PassBundle server, unlock the vault kept on this
// device, and use it, with the look every platform shares. It locks by itself
// as chosen in Settings (docs/core.md, "Locking by itself"). Several accounts
// can be signed in side by side (accounts.dart), each locking on its own; the
// screens show one at a time. In a build with
// test hooks (core.dart, testHooks; never a release), PB_MEASURE=1 unlocks by
// itself (with PB_MEASURE_PASSWORD) and reports timings (tools/measure.sh).
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import 'accounts.dart';
import 'core.dart';
import 'sign_in.dart';
import 'theme.dart';
import 'updates.dart';
import 'vault.dart';
import 'window_chrome.dart';

final measuring = testSetting('PB_MEASURE') == '1';
var _readyReported = false;
final _measureFile = testSetting('PB_MEASURE_FILE');

/// A timing line for tools/measure.sh: to PB_MEASURE_FILE when set (a Windows
/// app has no stdout), and to stdout where there is one.
void report(String line) {
  final file = _measureFile;
  if (file != null) {
    File(file).writeAsStringSync('$line\n', mode: FileMode.append, flush: true);
  }
  try {
    stdout.writeln(line);
  } on Exception {
    // No console.
  }
}

Future<void> main(List<String> args) async {
  // Duo's way back on Windows and Linux: the OS opened this second copy of
  // the app with the link, which goes to the copy that's signing in
  // (two_step.dart); this one ends before it shows.
  if ((Platform.isWindows || Platform.isLinux) &&
      args.isNotEmpty &&
      args.first.startsWith('passbundle://')) {
    Core().handOverLink(args.first);
    exit(0);
  }
  WidgetsFlutterBinding.ensureInitialized();
  // The app under see-through status and navigation bars, on every version
  // of Android as Android 15 and later draw it, so the ground runs under
  // them (SystemBars, theme.dart).
  if (Platform.isAndroid) {
    unawaited(SystemChrome.setEnabledSystemUIMode(SystemUiMode.edgeToEdge));
  }
  await Core.openOnThisDevice();
  // How the window's top is drawn on a computer (window_chrome.dart), known
  // before the first frame, so the app opens as it stays.
  await WindowFrameKeeper.instance.load();
  // Out of screenshots and screen sharing unless the person allowed them
  // (window_chrome.dart): the runner keeps the window out until it hears.
  if (keepsOutOfScreenshots) {
    await allowScreenshots(Core().screenshotsAllowed());
  }
  if (measuring) {
    SchedulerBinding.instance.addPostFrameCallback((_) {
      report('first-frame ${DateTime.now().microsecondsSinceEpoch}');
    });
  }
  // The tray, and Windows Hello with it, come from the desktop agent.
  if (!measuring && (Platform.isWindows || Platform.isMacOS)) {
    Core.startAgent();
  }
  // A newer release, at most once a day; the vault says so quietly. Not
  // while measuring (tools/measure.sh), which stays offline.
  if (!measuring) checkForUpdatesOnStart();
  // On Windows and Linux, an update waiting for this version to start hears
  // it did, once it's drawn (updates.dart).
  SchedulerBinding.instance.addPostFrameCallback((_) => updateStarted());
  runApp(const PassBundleApp());
}

class PassBundleApp extends StatefulWidget {
  const PassBundleApp({super.key, this.themeMode});

  /// Light, dark, or (when null) PB_THEME, else following the system.
  final ThemeMode? themeMode;

  @override
  State<PassBundleApp> createState() => _PassBundleAppState();
}

class _PassBundleAppState extends State<PassBundleApp> {
  late ThemeMode mode =
      widget.themeMode ??
      switch (testSetting('PB_THEME')) {
        'dark' => ThemeMode.dark,
        'light' => ThemeMode.light,
        _ => ThemeMode.system,
      };
  final core = Core();

  /// The accounts signed in here, and the one shown (`Core.accounts`).
  late Map<String, dynamic> accounts;

  /// The shown account's state (or, with none, the sign-in screen's).
  late Map<String, dynamic> state;

  /// Signing in again while a vault is kept here (the server signed this device out).
  bool signingInAgain = false;

  /// Adding another account: the sign-in screen, and back to the one shown.
  bool adding = false;

  /// What the vault opens to show, when the screens switched to its account
  /// to show it (an item picked in "All accounts", say), and for which
  /// screen: dropped once that screen is gone.
  VaultStart? start;
  String? startScreen;

  /// Unlocked just now with the master password (signing in, or the lock
  /// screen's field), not biometrics: a phone may offer biometrics for next
  /// time (unlock_offer.dart).
  bool withPassword = false;

  /// Asks the phone's lock screen to start the biometric prompt by itself
  /// (sign_in.dart, `UnlockScreen.promptBiometric`): as the app opens
  /// locked, and each time it comes back to the front locked, never after
  /// the person pressed Lock or while it's in use. Counted against how many
  /// the lock screen has taken, so one never waits for a later lock.
  int biometricAsks = 1;
  int biometricAsksTaken = 0;
  bool get promptBiometric => biometricAsks > biometricAsksTaken;

  /// While unlocked, asks the core every two seconds whether to lock by itself.
  Timer? ticker;
  DateTime lastUsed = DateTime(0);

  /// The native runner's word that the computer is about to sleep.
  static const computer = MethodChannel('passbundle/computer');

  /// Leaving the screen and coming back (on phones, the app switcher and home).
  late final AppLifecycleListener lifecycle;

  /// Each state's pages, for the window's buttons (window_chrome.dart): each
  /// state has a MaterialApp, and so a Navigator, of its own.
  final pages = <Object?, WindowPages>{};

  /// The screen shown: its account, and its state.
  String screen = '';

  /// Where asking before signing out opens: over the screen shown, by its
  /// Navigator (each screen's own, so nothing open over one moves to the
  /// next).
  BuildContext? get shownContext => pages[screen]?.navigator?.overlay?.context;

  @override
  void initState() {
    super.initState();
    // The account shown last, as the core opened them.
    accounts = core.accounts();
    Core.account = accounts['active'] as String? ?? '';
    state = core.state();
    // Opened unlocked (Android's autofill unlocked it in this process): no
    // prompt waits for a later lock.
    if (state['state'] != 'locked') biometricAsksTaken = biometricAsks;
    computer.setMethodCallHandler(fromComputer);
    HardwareKeyboard.instance.addHandler(keyPressed);
    // An account's name or color changed (Settings): the menus show it.
    accountsChanged.addListener(refresh);
    lifecycle = AppLifecycleListener(
      // "Immediately" locks as the app leaves the screen: each account whose
      // choice, or policy, that is.
      onHide: () {
        if (anyUnlocked && core.left()['locked'] == true) refresh();
      },
      // Back: check at once, before the vault is in front of anyone. Android's
      // autofill may have unlocked an account meanwhile (it runs in this
      // process), and then the vault shows, if its time isn't up.
      onShow: () {
        if (seen(core.accounts()) != seen(accounts)) refresh();
        if (anyUnlocked) tick();
        // Back locked: the phone's lock screen asks for the biometric.
        if (!adding && core.state()['state'] == 'locked') {
          setState(() => biometricAsks = biometricAsksTaken + 1);
        }
      },
    );
    watch();
  }

  /// Whether any account is unlocked: then the core is asked whether to lock.
  bool get anyUnlocked => (accounts['accounts'] as List? ?? const []).any(
    (a) => (a as Map)['state'] == 'unlocked',
  );

  /// The accounts and their states, to tell whether anything changed.
  static String seen(Map<String, dynamic> accounts) => [
    accounts['active'],
    for (final a in (accounts['accounts'] as List? ?? const []))
      '${(a as Map)['id']} ${a['state']}',
  ].join(',');

  @override
  void dispose() {
    accountsChanged.removeListener(refresh);
    lifecycle.dispose();
    ticker?.cancel();
    HardwareKeyboard.instance.removeHandler(keyPressed);
    computer.setMethodCallHandler(null);
    super.dispose();
  }

  void refresh({bool withPassword = false}) {
    setState(() {
      accounts = core.accounts();
      // The one the core shows, unless another is being added.
      if (!adding) Core.account = accounts['active'] as String? ?? '';
      state = core.state();
      signingInAgain = false;
      this.withPassword = withPassword;
      // Shown unlocked (or signed out): an ask from before is done with.
      if (state['state'] != 'locked') biometricAsksTaken = biometricAsks;
    });
    watch();
  }

  /// Shows another account, unlocked or not; the one shown stays as it is.
  void switchTo(String id) {
    core.switchTo(id);
    start = null;
    refresh();
  }

  /// Shows another account's vault, opening what was picked from it there
  /// (an item from "All accounts", a new item in the account chosen for it).
  void openIn(String id, VaultStart where) {
    if (core.switchTo(id).ok) {
      start = where;
      startScreen = '$id unlocked';
    }
    refresh();
  }

  /// Adding another account: the sign-in screen, with the way back.
  void add() {
    setState(() {
      adding = true;
      Core.account = '';
      state = core.state();
    });
  }

  /// Back from adding an account (or signing in again) to the one shown.
  void cancelSigningIn() {
    core.cancelSignIn();
    adding = false;
    refresh();
  }

  /// Signs out of the account shown, once the person says so; the next one
  /// shows, or the sign-in screen.
  Future<void> signOut(BuildContext context) async {
    final status = await Core.biometricStatus();
    if (!context.mounted) return;
    final several = (accounts['accounts'] as List? ?? const []).length > 1;
    final sure = await askToSignOut(
      context,
      method: status['status'] == 'setUp' ? status['method'] as String? : null,
      email: state['email'] as String?,
      others: several,
    );
    if (!sure) return;
    core.signOut();
    refresh();
  }

  void watch() {
    final unlocked = anyUnlocked;
    if (unlocked) {
      ticker ??= Timer.periodic(const Duration(seconds: 2), (_) => tick());
    } else {
      ticker?.cancel();
      ticker = null;
    }
  }

  /// Asks the core whether to lock: any account that locked just now, or
  /// that locked or unlocked some other way (Android's autofill, say),
  /// shows as it is.
  void tick() {
    final ticked = core.tick();
    if (ticked['busy'] == true) return;
    if (ticked['locked'] == true || seen(ticked) != seen(accounts)) refresh();
  }

  Future<void> fromComputer(MethodCall call) async {
    if (call.method == 'sleeping' &&
        anyUnlocked &&
        core.sleeping()['locked'] == true) {
      refresh();
    }
    // The Mac's PassBundle menu, Settings… (Command-comma): the vault
    // shown opens them; locked, there's nothing to set yet.
    if (call.method == 'settings' && state['state'] == 'unlocked') {
      settingsAsked.value++;
    }
  }

  /// The person used the app: the core hears it at most once a second.
  void used() {
    final now = DateTime.now();
    if (state['state'] != 'unlocked' ||
        now.difference(lastUsed) < const Duration(seconds: 1)) {
      return;
    }
    lastUsed = now;
    core.used();
  }

  bool keyPressed(KeyEvent event) {
    used();
    return false;
  }

  @override
  Widget build(BuildContext context) {
    final server = state['server'] as String?;
    final email = state['email'] as String?;
    final menu = AccountsMenu.from(
      accounts,
      onSwitch: switchTo,
      onAdd: add,
      onSignOut: () {
        final context = shownContext;
        if (context != null) signOut(context);
      },
    );
    final Widget home;
    var vault = false;
    if (adding) {
      home = SignInScreen(
        adding: true,
        onSignedIn: () {
          adding = false;
          refresh(withPassword: true);
        },
        onCancel: cancelSigningIn,
      );
    } else if (state['state'] == 'signedOut' || signingInAgain) {
      home = SignInScreen(
        server: server,
        email: email,
        onSignedIn: () => refresh(withPassword: true),
        onCancel: signingInAgain ? cancelSigningIn : null,
      );
    } else if (state['state'] == 'locked') {
      home = UnlockScreen(
        email: email!,
        server: server!,
        menu: menu,
        promptBiometric: promptBiometric,
        onPrompted: () => setState(() => biometricAsksTaken = biometricAsks),
        onUnlocked: (ms) {
          if (measuring) {
            report('unlocked ${ms ?? 0} ms, ${core.list('').length} items');
          }
          refresh(withPassword: ms != null);
        },
        onSignOut: () {
          core.signOut();
          refresh();
        },
      );
    } else {
      vault = true;
      home = VaultScreen(
        key: ValueKey('vault ${Core.account}'),
        core: core,
        email: email!,
        server: server,
        menu: menu,
        mode: mode,
        onMode: (m) => setState(() => mode = m),
        onLock: () {
          core.lock();
          // Locked by the person: the lock screen waits for them.
          biometricAsksTaken = biometricAsks;
          refresh();
        },
        onSignInAgain: () => setState(() => signingInAgain = true),
        // Measuring uses a sample vault with no server behind it.
        syncOnOpen: !measuring,
        offerBiometric: withPassword,
        start: start,
        onOpenIn: openIn,
      );
      if (measuring && !_readyReported) {
        _readyReported = true;
        WidgetsBinding.instance.addPostFrameCallback(
          (_) => Timer(const Duration(seconds: 1), () {
            // How big the app's view is, in points: the window tests
            // (tools/mac-window-test.sh) check it's the whole window, with
            // no title bar over it.
            final view = WidgetsBinding.instance.platformDispatcher.views.first;
            final size = view.physicalSize / view.devicePixelRatio;
            report('view ${size.width.round()}x${size.height.round()}');
            report('ready');
          }),
        );
      }
    }
    // Each account's each state has its own: switching accounts closes
    // whatever was open over the one shown, as locking does.
    screen = '${adding ? 'adding' : Core.account} ${state['state']}';
    final shown = pages.putIfAbsent(screen, WindowPages.new);
    if (screen != startScreen) start = null;
    return Listener(
      onPointerDown: (_) => used(),
      onPointerMove: (_) => used(),
      onPointerHover: (_) => used(),
      onPointerSignal: (_) => used(),
      child: MaterialApp(
        // A new one for each account's each state: locking, or showing
        // another account, closes whatever was open over the vault (an
        // item's editor, a menu) at once, with no fade.
        key: ValueKey(screen),
        title: 'PassBundle',
        navigatorObservers: [shown],
        debugShowCheckedModeBanner: false,
        theme: passBundleTheme(Brightness.light),
        darkTheme: passBundleTheme(Brightness.dark),
        themeMode: mode,
        // The ground behind everything too, so switching between light and
        // dark never shows what's behind the app. On a computer, the app
        // draws the window's top: the wide vault's band, or a strip.
        builder: (context, child) => SystemBars(
          child: Ground(
            child: WindowChrome(
              integrated: vault && isWide(context),
              pages: shown,
              child: child!,
            ),
          ),
        ),
        home: home,
      ),
    );
  }
}
