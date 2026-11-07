package com.passbundle.app

/**
 * What autofill does for the fields it's asked about, and how it unlocks
 * (docs/phones.md, "Autofill"): plain decisions, kept apart from Android's
 * classes so they're tested on the computer (src/test, UnlockingTest).
 */
object Unlocking {
    /** What autofill offers: nothing while the core is busy, signing in with no account here, unlocking while locked, or the logins. */
    enum class Offer { NOTHING, SIGN_IN, UNLOCK, LOGINS }

    /** From what the core says (Core.fillable): `busy`, `signedOut`, and `locked`. */
    fun offer(busy: Boolean, signedOut: Boolean, locked: Boolean): Offer =
        when {
            busy -> Offer.NOTHING
            signedOut -> Offer.SIGN_IN
            locked -> Offer.UNLOCK
            else -> Offer.LOGINS
        }

    /** How a locked account unlocks for autofill: its biometric unlock when it's on, else its master password in PassBundle's own dialog. */
    enum class Way { BIOMETRIC, MASTER_PASSWORD }

    /** From the account's biometric unlock (Biometric.status): "setUp" is the only one that unlocks. */
    fun way(biometric: String): Way = if (biometric == "setUp") Way.BIOMETRIC else Way.MASTER_PASSWORD

    /** What the master password dialog says, naming the account when there's more than one on the phone. */
    fun message(accountLabel: String?, why: String? = null): String {
        val ask =
            if (accountLabel.isNullOrBlank()) {
                "Enter your master password to fill this in."
            } else {
                "Enter the master password for $accountLabel to fill this in."
            }
        return if (why.isNullOrBlank()) ask else "$why\n\n$ask"
    }
}
