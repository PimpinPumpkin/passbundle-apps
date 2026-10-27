#import "GeneratedPluginRegistrant.h"

#include <stddef.h>
#include <stdint.h>

// The core's calls for the app's Swift (core/src/ffi.rs). The core is linked
// into the app (app/ios/PassBundleCore.xcframework, tools/ios-core.sh).

// An item's value for the pasteboard, as UTF-8; null while locked or busy, or
// if there's nothing to copy. Free it with pb_free_secret, which wipes it.
uint8_t *pb_secret(const char *id, const char *what, size_t *len);
void pb_free_secret(uint8_t *bytes, size_t len);

// Unlocking with Face ID or Touch ID: the user key, while unlocked, for the
// Keychain (free it with pb_free_secret); opening the kept vault with it, ""
// or the words to show (free it with pb_free); and Swift's function that
// turns it off, which the core calls when signing out.
uint8_t *pb_user_key(size_t *len);
char *pb_unlock_with_key(const uint8_t *key, size_t len);
char *pb_confirm_approval_with_key(const uint8_t *key, size_t len);
void pb_set_remove_biometric(void (*remove)(void));
// Saves an attachment in `dir` (the app's Documents folder): {"ok", "path"} or the words to show.
char *pb_save_attachment_in(const char *id, const char *attachment_id, const char *dir);
void pb_free(char *ptr);
