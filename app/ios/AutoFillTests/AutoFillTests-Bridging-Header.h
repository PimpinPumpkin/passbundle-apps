// The extension's calls into the core (AutoFill-Bridging-Header.h), and the
// one more the tests use to keep an account's key as the app keeps it for
// Face ID: its user key, while it's unlocked (free it with pb_free_secret).
#include "../AutoFill/AutoFill-Bridging-Header.h"

uint8_t *pb_user_key(const char *account, size_t *len);
