#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// The core's calls for the AutoFill extension's Swift (core/src/ffi.rs). The
// core is linked into the extension as into the app
// (app/ios/PassBundleCore.xcframework, tools/ios-core.sh). Answers that are
// text are JSON; free them with pb_free. Each call about an account names it
// by its id on this device (`account`).

// Opens the accounts the app keeps in `dir` (the App Group's folder), all
// locked, read beside the app without changing anything there: as pb_accounts,
// {active, accounts: [{id, email, name, state}]}.
char *pb_autofill_open(const char *dir);
// Locks every account and lets them go, as a request ends.
char *pb_autofill_close(void);
// Unlocks an account with its master password: {"ok": true} or the words to show.
char *pb_unlock(const char *account, const char *password);
// The logins for a website, from every account unlocked here:
// {locked, account, items: [{id, account, accountLabel, name, username, reprompt}]},
// never a password.
char *pb_fillable(const char *domain);
// Every unlocked account's items whose name, username, or website has
// `query`: [{id, account, kind, name, subtitle, reprompt, ...}].
char *pb_list_all(const char *query);
// One of an account's items, as its screen shows it before anything is
// revealed: {name, website, uris: [its websites], ...}, never a secret. The
// extension reads a login's websites from it, to ask before filling a login
// on a site that isn't one of them.
char *pb_item(const char *account, const char *id);
// A login's "username" or "password" to fill, as UTF-8; null if there's none.
// Free it with pb_free_secret, which wipes it.
uint8_t *pb_fill_secret(const char *account, const char *id, const char *what, size_t *len);
void pb_free_secret(uint8_t *bytes, size_t len);
// Opens an account's kept vault with the user key from its Keychain item, or
// compares it with the open vault's to approve a sign-in (the app's): "" or
// the words to show.
char *pb_unlock_with_key(const char *account, const uint8_t *key, size_t len);
char *pb_confirm_approval_with_key(const char *account, const uint8_t *key, size_t len);
// Swift's function that turns an account's Face ID off, which the core calls
// when a kept key no longer opens its vault.
void pb_set_remove_biometric(void (*remove)(const char *account));
// The master password, for a login that asks for it again before it fills:
// {"ok": true} or the words to show.
char *pb_confirm_master_password(const char *account, const char *id, const char *password);
// A site's passkeys, from every account unlocked here (IDs in base64url):
// {locked, account, accounts: [{id, label}], items: [{id, account, accountLabel,
// name, userName, credentialId, userHandle, discoverable, reprompt}]}, never a key.
char *pb_passkeys(const char *rp_id);
// Signs in to a site with a login's passkey, from the account named:
// {ok, authenticatorData, signature, credentialId, userHandle} or the words to show.
char *pb_passkey_assert(const char *account, const char *id, const char *rp_id, const char *client_data_hash,
                        const char *user_verified);
// Makes a passkey in a new login, from {account, rpId, userHandle, userName,
// userVerified, discoverable, exclude}, saved to the server first:
// {ok, itemId, credentialId, attestationObject} or the words to show.
char *pb_passkey_create(const char *request);
void pb_free(char *ptr);
