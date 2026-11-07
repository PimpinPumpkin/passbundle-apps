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
// Saves an attachment in `dir` (the app's Documents folder): {"ok", "path"} or the words to show.
char *pb_save_attachment_in(const char *account, const char *id, const char *attachment_id, const char *dir);
void pb_free(char *ptr);
