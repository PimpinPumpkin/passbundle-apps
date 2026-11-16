#import "GeneratedPluginRegistrant.h"

#include <stddef.h>
#include <stdint.h>

// The core's calls for the app's Swift (core/src/ffi.rs). The core is linked
// into the app (app/ios/PassBundleCore.xcframework, tools/ios-core.sh).

// Each call about an account names it by its id on this device (`account`),
// the one the screens show (core.dart, Core.account).

// An item's value for the pasteboard, as UTF-8; null while locked or busy, or
// if there's nothing to copy. Free it with pb_free_secret, which wipes it.
uint8_t *pb_secret(const char *account, const char *id, const char *what, size_t *len);
void pb_free_secret(uint8_t *bytes, size_t len);

// Unlocking an account with Face ID or Touch ID: its user key, while it's
// unlocked, for its own Keychain item (free it with pb_free_secret); opening
// its kept vault with it, "" or the words to show (free it with pb_free); and
// Swift's function that turns an account's off, which the core calls when
// signing out of it.
uint8_t *pb_user_key(const char *account, size_t *len);
char *pb_unlock_with_key(const char *account, const uint8_t *key, size_t len);
char *pb_confirm_approval_with_key(const char *account, const uint8_t *key, size_t len);
void pb_set_remove_biometric(void (*remove)(const char *account));
// Unlocking with a PIN: the app's seal for what the core keeps, and the
// opening of it (DeviceSeal.swift), given once at start. Each reads
// `input_len` bytes at `input` for the account, writes at most `capacity`
// bytes to `output`, and returns how many it wrote, or a negative number.
typedef intptr_t (*pb_device_seal_call)(
    const char *account, const uint8_t *input, size_t input_len, uint8_t *output, size_t capacity);
void pb_set_device_seal(pb_device_seal_call seal, pb_device_seal_call open);
// Saves an attachment in `dir` (the app's Documents folder): {"ok", "path"} or the words to show.
char *pb_save_attachment_in(const char *account, const char *id, const char *attachment_id, const char *dir);
void pb_free(char *ptr);

// What iOS may suggest above the keyboard, from every unlocked account:
// [{account, id, username, domain}], never a password (free it with pb_free).
char *pb_suggestions(void);
// And the passkeys iOS may list in its passkey sheet, from every unlocked
// account: [{account, id, rpId, userName, credentialId, userHandle}], never a
// key (free it with pb_free).
char *pb_passkey_suggestions(void);
