// Unlocking with a PIN (pin.dart, and the lock screen in sign_in.dart),
// against the real core (PB_CORE_LIB), whose test store stands in for the
// device's seal (Core.useDeviceSeal): set up in Settings with the master password again, then
// the lock screen asks for the PIN, with the way back to the master
// password; 5 wrong tries turn it off and say so; an organization's
// "Remove Unlock with PIN" policy turns the setting off with why; a device
// that can't keep a PIN doesn't offer one; and a vault a PIN opened can't
// turn on biometrics, or bring up the phone's offer of them.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/core.dart';
import 'package:passbundle/main.dart';
import 'package:passbundle/unlock_offer.dart';

import 'support.dart';

const pin = '2580';

/// Settings' switch for it, and the lock screen's field.
const pinSwitch = ValueKey('pin');
const pinField = ValueKey('unlock-pin');

/// The switch for unlocking with biometrics, in Settings.
const biometric = ValueKey('biometric');

const turnedOff =
    'Unlocking with a PIN was turned off after 5 wrong tries. Use your master password.';

Future<void> computerSize(WidgetTester tester) async {
  tester.view.physicalSize = const Size(1280, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

/// Unlocks with the master password, and waits out the sync on opening
/// (there's no server behind the sample).
Future<void> unlockWithPassword(
  WidgetTester tester, {
  bool phone = false,
}) async {
  await tester.enterText(find.byType(TextField), samplePassword);
  await tester.tap(find.text('Unlock'));
  await pumpUntil(tester, find.text(phone ? 'Logins' : 'Example Mail'));
  await pumpUntil(tester, find.textContaining('Sign in again to get changes'));
  await tester.pumpAndSettle();
}

/// Settings, once it shows the PIN's row ([pinRow]: where the device can
/// keep one), or once it's had the time to.
Future<void> openSettings(WidgetTester tester, {bool pinRow = true}) async {
  await tester.tap(find.byTooltip('Settings'));
  await tester.pump();
  await pumpUntil(tester, find.text('Lock after'));
  await pumpUntilGone(tester, find.byKey(const ValueKey('unlock-loading')));
  if (pinRow) {
    await pumpUntil(tester, find.byKey(pinSwitch));
  } else {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(seconds: 1)),
    );
  }
  await tester.pump(const Duration(milliseconds: 400));
}

Future<void> closeSettings(WidgetTester tester) async {
  await tester.tap(find.text('Close'));
  await tester.pumpAndSettle();
}

Finder pinSwitchedTo(bool on) => find.byWidgetPredicate(
  (w) => w is SwitchListTile && w.key == pinSwitch && w.value == on,
);

/// Settings' sheet, filled in and sent.
Future<void> fillSetup(
  WidgetTester tester, {
  String password = samplePassword,
  String newPin = pin,
  String? again,
}) async {
  await tester.enterText(
    find.byKey(const ValueKey('pin-master-password')),
    password,
  );
  await tester.enterText(find.byKey(const ValueKey('pin-new')), newPin);
  await tester.enterText(
    find.byKey(const ValueKey('pin-again')),
    again ?? newPin,
  );
  await tester.tap(find.text('Set PIN'));
  await tester.pump();
}

/// Sets the PIN from Settings, which then shows it on.
Future<void> turnOn(WidgetTester tester) async {
  await openSettings(tester);
  await tester.tap(find.byKey(pinSwitch));
  await tester.pumpAndSettle();
  await fillSetup(tester);
  await pumpUntil(tester, pinSwitchedTo(true));
  await closeSettings(tester);
}

/// Locks (a computer's sidebar has Lock; a phone's bar, its icon), with a
/// PIN to unlock with.
Future<void> lock(WidgetTester tester, {bool phone = false}) async {
  await tester.tap(phone ? find.byTooltip('Lock') : find.text('Lock'));
  await tester.pump();
  await pumpUntil(tester, find.byKey(pinField));
  await tester.pumpAndSettle();
}

Future<void> tryPin(WidgetTester tester, String typed) async {
  await tester.enterText(find.byKey(pinField), typed);
  await tester.tap(find.text('Unlock'));
  await tester.pump();
  await pumpUntilGone(tester, find.byType(CircularProgressIndicator));
}

/// The account's file, which holds no PIN, in its folder (`accounts/<id>`),
/// if it's there.
File? pinFile(String dir) {
  final found = Directory(dir)
      .listSync(recursive: true)
      .whereType<File>()
      .where((f) => f.uri.pathSegments.last == 'pin.json');
  return found.isEmpty ? null : found.single;
}

void main() {
  setUpAll(loadFonts);
  // The core's test store stands in for a device that can keep a PIN, for
  // each test here; the app's other tests have none.
  setUp(() => Core().useDeviceSeal(true));
  tearDown(() => Core().useDeviceSeal(false));

  testWidgets(
    'set up in Settings with the master password again, the lock screen unlocks with the PIN',
    (tester) async {
      await computerSize(tester);
      final dir = openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      await openSettings(tester);
      expect(find.text('Unlock with a PIN'), findsOneWidget);
      expect(
        find.text('Instead of your master password, on this device'),
        findsOneWidget,
      );
      expect(pinSwitchedTo(false), findsOneWidget);

      // Cancelled: nothing's set.
      await tester.tap(find.byKey(pinSwitch));
      await tester.pumpAndSettle();
      expect(find.text('Set PIN'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(pinSwitchedTo(false), findsOneWidget);

      await tester.tap(find.byKey(pinSwitch));
      await tester.pumpAndSettle();
      // Too short, and the two not the same: the sheet says so, and the
      // core isn't asked.
      await fillSetup(tester, newPin: '258');
      expect(find.text('A PIN needs at least 4 characters.'), findsOneWidget);
      await fillSetup(tester, again: '2581');
      expect(find.text("The PINs don't match."), findsOneWidget);
      // A wrong master password: the core refuses it, and the fields are
      // cleared.
      await fillSetup(tester, password: 'not the master password');
      await pumpUntil(tester, find.text("That master password isn't right."));
      expect(find.text('Set PIN'), findsOneWidget);
      expect(pinFile(dir), isNull);
      for (final field in ['pin-master-password', 'pin-new', 'pin-again']) {
        final text = find.descendant(
          of: find.byKey(ValueKey(field)),
          matching: find.byType(EditableText),
        );
        expect(tester.widget<EditableText>(text).controller.text, isEmpty);
      }
      await fillSetup(tester);
      await pumpUntil(tester, pinSwitchedTo(true));
      expect(pinFile(dir), isNotNull);
      expect(pinFile(dir)!.readAsStringSync(), isNot(contains(pin)));
      await closeSettings(tester);

      // Locked: the PIN's field, the way to the master password, and no
      // hint for a master password that isn't asked for.
      await lock(tester);
      expect(find.text('PIN'), findsOneWidget);
      expect(find.text('Use your master password instead'), findsOneWidget);
      expect(find.text('Get your master password hint'), findsNothing);
      await tryPin(tester, '2581');
      expect(find.text("That PIN isn't right. 4 tries left."), findsOneWidget);
      // The field is cleared, as the master password's is.
      expect(
        tester
            .widget<EditableText>(
              find.descendant(
                of: find.byKey(pinField),
                matching: find.byType(EditableText),
              ),
            )
            .controller
            .text,
        isEmpty,
      );
      await tryPin(tester, pin);
      await pumpUntil(tester, find.text('Example Mail'));

      // Off again, after asking: the lock screen asks for the master password.
      await openSettings(tester);
      await tester.tap(find.byKey(pinSwitch));
      await tester.pumpAndSettle();
      expect(find.text('Turn off unlocking with a PIN?'), findsOneWidget);
      await tester.tap(find.text('Turn off'));
      await pumpUntil(tester, pinSwitchedTo(false));
      expect(pinFile(dir), isNull);
      await closeSettings(tester);
      await tester.tap(find.text('Lock'));
      await tester.pump();
      await pumpUntil(tester, find.text('Unlock PassBundle'));
      // Long enough for the core to have said whether there's a PIN.
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(seconds: 1)),
      );
      await tester.pumpAndSettle();
      expect(find.byKey(pinField), findsNothing);
      expect(find.text('Use your PIN instead'), findsNothing);
    },
  );

  testWidgets(
    'the master password still unlocks, and the lock screen goes back to the PIN',
    (tester) async {
      await computerSize(tester);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      await turnOn(tester);
      await lock(tester);

      await tester.tap(find.text('Use your master password instead'));
      await tester.pumpAndSettle();
      expect(find.text('Master password'), findsOneWidget);
      expect(find.byKey(pinField), findsNothing);
      await tester.tap(find.text('Use your PIN instead'));
      await tester.pumpAndSettle();
      expect(find.byKey(pinField), findsOneWidget);
      await tester.tap(find.text('Use your master password instead'));
      await tester.pumpAndSettle();
      await unlockWithPassword(tester);
      expect(find.text('Example Mail'), findsOneWidget);
    },
  );

  testWidgets(
    'the fifth wrong PIN turns it off, and the lock screen says the master password is needed',
    (tester) async {
      await computerSize(tester);
      final dir = openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      await turnOn(tester);
      await lock(tester);

      for (final left in ['4 tries', '3 tries', '2 tries', '1 try']) {
        await tryPin(tester, '0000');
        expect(find.text("That PIN isn't right. $left left."), findsOneWidget);
      }
      await tryPin(tester, '0000');
      // The master password's field, with why, and no way back to a PIN.
      expect(find.byKey(pinField), findsNothing);
      expect(find.text('Master password'), findsOneWidget);
      expect(find.text(turnedOff), findsOneWidget);
      expect(find.text('Use your PIN instead'), findsNothing);
      expect(pinFile(dir), isNull);

      await unlockWithPassword(tester);
      await openSettings(tester);
      expect(pinSwitchedTo(false), findsOneWidget);
    },
  );

  testWidgets(
    'an organization that removes unlocking with a PIN turns the setting off, and says why',
    (tester) async {
      await computerSize(tester);
      openSample(
        policies: [
          {
            'id': 'pin-policy',
            'organizationId': 'another-organization',
            'type': 14,
            'enabled': true,
            'data': null,
            'object': 'policy',
          },
        ],
      );
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      await openSettings(tester);
      expect(
        find.text("Your organization doesn't allow unlocking with a PIN"),
        findsOneWidget,
      );
      expect(pinSwitchedTo(false), findsOneWidget);
      expect(
        tester.widget<SwitchListTile>(find.byKey(pinSwitch)).onChanged,
        isNull,
      );
    },
  );

  testWidgets(
    'a device that can\'t keep a PIN (Linux, a Mac without a Secure Enclave) doesn\'t offer one',
    (tester) async {
      await computerSize(tester);
      Core().useDeviceSeal(false);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      await openSettings(tester, pinRow: false);
      expect(find.byKey(pinSwitch), findsNothing);
      expect(find.textContaining('PIN'), findsNothing);
    },
  );

  testWidgets(
    'in a vault a PIN opened, the Mac\'s Touch ID can\'t be turned on until the master password unlocks it',
    (tester) async {
      await computerSize(tester);
      final mac = PhoneStandIn(tester, mac: true);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester);
      await turnOn(tester);
      await lock(tester);
      await tryPin(tester, pin);
      await pumpUntil(tester, find.text('Example Mail'));

      await openSettings(tester);
      final touchID = tester.widget<SwitchListTile>(find.byKey(biometric));
      expect(touchID.value, isFalse);
      expect(touchID.onChanged, isNull);
      expect(
        find.text('Unlock with your master password to turn this on'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(biometric));
      await tester.pumpAndSettle();
      expect(mac.enrolled, 0);
      await closeSettings(tester);

      // The master password: then it can.
      await lock(tester);
      await tester.tap(find.text('Use your master password instead'));
      await tester.pumpAndSettle();
      await unlockWithPassword(tester);
      await openSettings(tester);
      expect(
        tester.widget<SwitchListTile>(find.byKey(biometric)).onChanged,
        isNotNull,
      );
      await tester.tap(find.byKey(biometric));
      await pumpUntil(
        tester,
        find.byWidgetPredicate(
          (w) => w is SwitchListTile && w.key == biometric && w.value,
        ),
      );
      expect(mac.enrolled, 1);
    },
  );

  testWidgets(
    'on a phone, a PIN unlock doesn\'t bring up the offer to unlock with a fingerprint',
    (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      addTearDown(() => offerClock = DateTime.now);
      const offer = 'Unlock with fingerprint next time?';
      final phone = PhoneStandIn(tester);
      openSample();
      await tester.pumpWidget(const PassBundleApp());
      await unlockWithPassword(tester, phone: true);
      // After the master password it's offered; not now.
      expect(find.text(offer), findsOneWidget);
      await tester.tap(find.text('Not now'));
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('Settings'));
      await tester.pump();
      await pumpUntil(tester, find.byKey(pinSwitch));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(pinSwitch));
      await tester.pumpAndSettle();
      await fillSetup(tester);
      await pumpUntil(tester, pinSwitchedTo(true));
      await tester.pumpAndSettle();

      // Back to the vault, where Lock is.
      await tester.tap(
        find.descendant(
          of: find.byType(NavigationBar),
          matching: find.text('Items'),
        ),
      );
      await tester.pumpAndSettle();

      // Past "Not now"'s 30 days: a PIN unlock still doesn't offer it.
      offerClock = () => DateTime.now().add(const Duration(days: 31));
      await lock(tester, phone: true);
      await tryPin(tester, pin);
      await pumpUntil(tester, find.text('Logins'));
      await tester.pumpAndSettle();
      expect(find.text(offer), findsNothing);
      expect(phone.enrolled, 0);

      // The master password does.
      await lock(tester, phone: true);
      await tester.tap(find.text('Use your master password instead'));
      await tester.pumpAndSettle();
      await unlockWithPassword(tester, phone: true);
      expect(find.text(offer), findsOneWidget);
    },
  );
}
