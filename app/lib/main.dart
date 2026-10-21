// PassBundle: sign in to a PassBundle server, unlock the vault kept on this
// device, and use it, with the look every platform shares. Set PB_MEASURE=1
// to unlock by itself (with PB_MEASURE_PASSWORD) and report timings
// (tools/measure.sh).
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

import 'core.dart';
import 'sign_in.dart';
import 'theme.dart';
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

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  if (measuring) {
    SchedulerBinding.instance.addPostFrameCallback((_) {
      report('first-frame ${DateTime.now().microsecondsSinceEpoch}');
    });
  }
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

  void refresh() => setState(() {
    state = core.state();
    signingInAgain = false;
  });

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
    return MaterialApp(
      title: 'PassBundle',
      debugShowCheckedModeBanner: false,
      theme: passBundleTheme(Brightness.light),
      darkTheme: passBundleTheme(Brightness.dark),
      themeMode: mode,
      home: home,
    );
  }
}
