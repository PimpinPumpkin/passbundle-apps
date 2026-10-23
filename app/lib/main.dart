// PassBundle: sign in to a PassBundle server, unlock the vault kept on this
// device, and use it, with the look every platform shares. It locks by itself
// as chosen in Settings (docs/core.md, "Locking by itself"). Set PB_MEASURE=1
// to unlock by itself (with PB_MEASURE_PASSWORD) and report timings
// (tools/measure.sh).
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import 'core.dart';
import 'sign_in.dart';
import 'theme.dart';
import 'updates.dart';
import 'vault.dart';

final measuring = Platform.environment['PB_MEASURE'] == '1';
var _readyReported = false;
final _measureFile = Platform.environment['PB_MEASURE_FILE'];

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

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await Core.openOnThisDevice();
  if (measuring) {
    SchedulerBinding.instance.addPostFrameCallback((_) {
      report('first-frame ${DateTime.now().microsecondsSinceEpoch}');
    });
  }
  // The tray, and Windows Hello with it, come from the desktop agent.
  if (!measuring && (Platform.isWindows || Platform.isMacOS)) {
    Core.startAgent();
  }
  // On Android, a newer release, at most once a day; the vault says so quietly.
  checkForUpdatesOnStart();
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
      switch (Platform.environment['PB_THEME']) {
        'dark' => ThemeMode.dark,
        'light' => ThemeMode.light,
        _ => ThemeMode.system,
      };
  final core = Core();
  late Map<String, dynamic> state = core.state();

  /// Signing in again while a vault is kept here (the server signed this device out).
  bool signingInAgain = false;

  /// While unlocked, asks the core every two seconds whether to lock by itself.
  Timer? ticker;
  DateTime lastUsed = DateTime(0);

  /// The native runner's word that the computer is about to sleep.
  static const computer = MethodChannel('passbundle/computer');

  /// Leaving the screen and coming back (on phones, the app switcher and home).
  late final AppLifecycleListener lifecycle;

  @override
  void initState() {
    super.initState();
    computer.setMethodCallHandler(fromComputer);
    HardwareKeyboard.instance.addHandler(keyPressed);
    lifecycle = AppLifecycleListener(
      // "Immediately" locks as the app leaves the screen.
      onHide: () {
        if (state['state'] == 'unlocked' && core.left()['locked'] == true) {
          refresh();
        }
      },
      // Back: check at once, before the vault is in front of anyone. Android's
      // autofill may have unlocked the core meanwhile (it runs in this
      // process), and then the vault shows, if its time isn't up.
      onShow: () {
        if (state['state'] == 'locked' && core.state()['state'] == 'unlocked') {
          refresh();
        }
        if (state['state'] == 'unlocked') tick();
      },
    );
    watch();
  }

  @override
  void dispose() {
    lifecycle.dispose();
    ticker?.cancel();
    HardwareKeyboard.instance.removeHandler(keyPressed);
    computer.setMethodCallHandler(null);
    super.dispose();
  }

  void refresh() {
    setState(() {
      state = core.state();
      signingInAgain = false;
    });
    watch();
  }

  void watch() {
    final unlocked = state['state'] == 'unlocked';
    if (unlocked) {
      ticker ??= Timer.periodic(const Duration(seconds: 2), (_) => tick());
    } else {
      ticker?.cancel();
      ticker = null;
    }
  }

  void tick() {
    if (core.tick()['locked'] == true) refresh();
  }

  Future<void> fromComputer(MethodCall call) async {
    if (call.method == 'sleeping' &&
        state['state'] == 'unlocked' &&
        core.sleeping()['locked'] == true) {
      refresh();
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
    final Widget home;
    if (state['state'] == 'signedOut' || signingInAgain) {
      home = SignInScreen(
        server: server,
        email: email,
        onSignedIn: refresh,
        onCancel: signingInAgain
            ? () => setState(() => signingInAgain = false)
            : null,
      );
    } else if (state['state'] == 'locked') {
      home = UnlockScreen(
        email: email!,
        server: server!,
        onUnlocked: (ms) {
          if (measuring) {
            report('unlocked $ms ms, ${core.list('').length} items');
          }
          refresh();
        },
        onSignOut: () {
          core.signOut();
          refresh();
        },
      );
    } else {
      home = VaultScreen(
        key: const ValueKey('vault'),
        core: core,
        email: email!,
        mode: mode,
        onMode: (m) => setState(() => mode = m),
        onLock: () {
          core.lock();
          refresh();
        },
        onSignInAgain: () => setState(() => signingInAgain = true),
        // Measuring uses a sample vault with no server behind it.
        syncOnOpen: !measuring,
      );
      if (measuring && !_readyReported) {
        _readyReported = true;
        WidgetsBinding.instance.addPostFrameCallback(
          (_) => Timer(const Duration(seconds: 1), () => report('ready')),
        );
      }
    }
    return Listener(
      onPointerDown: (_) => used(),
      onPointerMove: (_) => used(),
      onPointerHover: (_) => used(),
      onPointerSignal: (_) => used(),
      child: MaterialApp(
        // A new one for each state: locking closes whatever was open over
        // the vault (an item's editor, a menu) at once, with no fade.
        key: ValueKey(state['state']),
        title: 'PassBundle',
        debugShowCheckedModeBanner: false,
        theme: passBundleTheme(Brightness.light),
        darkTheme: passBundleTheme(Brightness.dark),
        themeMode: mode,
        home: home,
      ),
    );
  }
}
