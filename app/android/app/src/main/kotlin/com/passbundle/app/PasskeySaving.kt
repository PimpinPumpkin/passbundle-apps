package com.passbundle.app

/**
 * Where a new passkey goes on Android, and what PasskeyActivity asks about
 * it (docs/phones.md, "Passkeys"), once the core has said which logins it
 * can go in (Core.passkeyLogins; matching.rs, `passkey_target`, has the
 * rules): plain decisions and words, kept apart from Android's classes so
 * they're tested on the computer (src/test, PasskeySavingTest).
 *
 * - **The site has one of PassBundle's passkeys already** (it named it among
 *   those it has, `excludeCredentials`): nothing is asked, and the site is
 *   told so (WebAuthn's InvalidStateError), never offered a replacement.
 * - **One login has the site's passkey for this account** (the same site ID
 *   and user handle): "Replace the passkey for <user> on <site>?", which
 *   says the old one stops working at the site's next sign-in.
 * - **Otherwise, logins for the site:** a list of them (each in its own
 *   account, labeled when there's more than one), then "New login" where
 *   an account unlocked here takes one (`Account.newLogin`); where none
 *   does, why, in the core's words, over the list instead.
 * - **None:** "Save a passkey in PassBundle?" for a new login, in an
 *   account that takes one; where none does (an organization keeps the
 *   person's items out of Personal and they have no shared folder to add
 *   to: core/src/app.rs, `no_new_passkey_login`), why, before anything is
 *   picked, and nothing is saved.
 */
object PasskeySaving {
    /** A login a new passkey can go in, as the core lists it. */
    data class Login(
        val id: String,
        /** Its account, by its id on this device. */
        val account: String,
        val name: String,
        val username: String? = null,
        /** With more than one account on the phone, its account's name or email. */
        val accountLabel: String? = null,
        /** It has the site's passkey for this account, which the new one replaces. */
        val replaces: Boolean = false,
        /** The account's name on the site, as the passkey it replaces has it. */
        val passkeyUserName: String? = null,
        /** It asks for the master password again before it's changed. */
        val reprompt: Boolean = false,
    )

    /** An account unlocked here, as the core lists them (Core.passkeyLogins' `accounts`). */
    data class Account(
        /** Its id on this device. */
        val id: String,
        /** Its name or email. */
        val label: String,
        /** A new login for the passkey can go in it (`newLogin`). */
        val newLogin: Boolean = true,
        /** Where one can't, why, in the core's words (`noNewLogin`). */
        val noNewLogin: String? = null,
    )

    /** What PasskeyActivity asks first. */
    sealed interface Step {
        /** The site has one already: it's told so, and nothing is asked. */
        data object Excluded : Step

        /** One login has this account's passkey for the site: replace it? */
        data class Replace(val login: Login) : Step

        /**
         * The logins it can go in, then "New login"; or, where no account
         * here takes a new login, why not ([noNewLogin]) instead.
         */
        data class Choose(val logins: List<Login>, val noNewLogin: String? = null) : Step

        /** None: a new login. */
        data object New : Step

        /** None, and no account here takes a new login: why, and nothing is saved. */
        data class Nowhere(val why: String) : Step
    }

    /**
     * From what the core says: whether the site has one already, the logins
     * it can go in, and why no account unlocked here takes a new login, if
     * none does ([noNewLogin]).
     */
    fun step(excluded: Boolean, logins: List<Login>, noNewLogin: String? = null): Step {
        val replacing = logins.filter { it.replaces }
        return when {
            excluded -> Step.Excluded
            // WebAuthn has an authenticator replace the account's old
            // passkey for the site. More than one (saved before PassBundle
            // replaced them) is the list's to choose from.
            replacing.size == 1 -> Step.Replace(replacing.single())
            logins.isNotEmpty() -> Step.Choose(logins, noNewLogin)
            noNewLogin == null -> Step.New
            else -> Step.Nowhere(noNewLogin)
        }
    }

    /**
     * Why no account unlocked here takes a new login, in the core's words
     * for the first of them (the one the screens show); null when one does.
     */
    fun noNewLogin(accounts: List<Account>): String? {
        if (accounts.isEmpty() || accounts.any { it.newLogin }) return null
        return accounts.first().noNewLogin ?: "PassBundle couldn't save the passkey."
    }

    /**
     * The accounts a new login can go in, of those unlocked here, and the
     * label to name it by when there's no choice to make: one, while
     * another here couldn't take it, so it's clear which.
     */
    fun newLoginIn(accounts: List<Account>): Pair<List<Account>, String?> {
        val takers = accounts.filter { it.newLogin }
        return takers to takers.singleOrNull()?.label?.takeIf { accounts.size > 1 }
    }

    const val NEW_LOGIN = "New login"

    /**
     * A login's row in the list: its name, then its username, that the new
     * passkey replaces its own, and its account's name when there's more
     * than one account.
     */
    fun row(login: Login): String {
        val under = listOfNotNull(login.username, "Replaces its passkey".takeIf { login.replaces }, login.accountLabel)
        return if (under.isEmpty()) login.name else "${login.name}\n${under.joinToString(" · ")}"
    }

    /** The list's rows: each login, then a new login where one can be made ([newLogin]). */
    fun rows(logins: List<Login>, newLogin: Boolean = true): List<String> =
        logins.map { row(it) } + listOfNotNull(NEW_LOGIN.takeIf { newLogin })

    /**
     * The site, as every question about a new passkey names it: its ID
     * first, which the passkey is bound to, then the name it gives itself
     * (anyone's to make up), and the account's name on it.
     */
    fun site(rpId: String, site: String, userName: String): String =
        "For $rpId${if (site == rpId || site.isEmpty()) "" else " (says it's $site)"}${if (userName.isEmpty()) "" else ", as $userName"}"

    /** "Save a passkey in PassBundle?" for a new login, in the account named when there's more than one. */
    fun newLogin(rpId: String, site: String, userName: String, accountLabel: String?): String =
        "${site(rpId, site, userName)}, in a new login${if (accountLabel == null) "" else " in $accountLabel"}."

    /**
     * "Save a passkey in PassBundle?" over the list of logins it can go in;
     * where no new login can be made, without one, and why ([noNewLogin]).
     */
    fun choose(rpId: String, site: String, userName: String, noNewLogin: String? = null): String =
        if (noNewLogin == null) {
            "${site(rpId, site, userName)}. Add it to one of your logins for this site, or to a new login."
        } else {
            "${site(rpId, site, userName)}. Add it to one of your logins for this site. $noNewLogin"
        }

    /** Who the passkey is for, in the question: the account's name on the site, as the site gives it or the old passkey has it. */
    fun who(userName: String, login: Login): String =
        userName.ifEmpty { login.passkeyUserName.orEmpty() }.ifEmpty { "this account" }

    fun replaceTitle(userName: String, login: Login, rpId: String): String = "Replace the passkey for ${who(userName, login)} on $rpId?"

    /** What replacing does: the old passkey goes, so it stops working at the site. */
    fun replaceMessage(login: Login, rpId: String): String =
        "${login.name}${if (login.accountLabel == null) "" else " (in ${login.accountLabel})"} has a passkey for this account already. " +
            "The new one takes its place, and the old one stops working the next time you sign in to $rpId."
}
