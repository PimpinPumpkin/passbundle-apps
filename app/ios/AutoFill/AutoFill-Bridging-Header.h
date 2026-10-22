#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// The core's calls for the AutoFill extension's Swift (core/src/ffi.rs). The
// core is linked into the extension as into the app
// (app/ios/PassBundleCore.xcframework, tools/ios-core.sh). Answers that are
// text are JSON; free them with pb_free.

// Opens the vault kept in `dir`, its state as JSON: signedOut, locked, unlocked.
char *pb_open(const char *dir, bool memory_secrets);
// Unlocks with the master password: {"ok": true} or the words to show.
char *pb_unlock(const char *password);
// Drops every key and everything decrypted.
char *pb_lock(void);
// The logins for a website: {locked, items: [{id, name, username}]}, never a password.
char *pb_fillable(const char *domain);
// Every item whose name, username, or website has `query`: [{id, kind, name, subtitle, ...}].
char *pb_list(const char *query);
// An item's value, as UTF-8; null if there's none. Free it with pb_free_secret, which wipes it.
uint8_t *pb_secret(const char *id, const char *what, size_t *len);
void pb_free_secret(uint8_t *bytes, size_t len);
// Opens the kept vault with the user key from the Keychain: "" or the words to show.
char *pb_unlock_with_key(const uint8_t *key, size_t len);
void pb_free(char *ptr);
