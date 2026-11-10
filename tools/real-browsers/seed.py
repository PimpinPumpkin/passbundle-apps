"""Real Chrome and Firefox, saving made-up passwords (fakes.json) in profiles
in a throwaway home folder, for the import's test against real browsers
(browsers/tests/real_browsers.rs). CI only, on a fresh Linux runner: it never
looks at anyone's own profiles.

    seed.py <root> fakes.json seed     # makes <root>/home and fills it
    seed.py <root> fakes.json check    # after the import turned saving off

Both browsers are whichever the runner has, so they change under this. Each
script asks the browser for what its own pages use, and a failure names the
browser's version and what it didn't have; the fix then is to look at how
that browser's own page does it now:

- Chrome saves through its password manager page's API
  (chrome://password-manager, chrome.passwordsPrivate), with its basic store
  (no keyring), as a Linux desktop without one has it, and its settings page
  reads its switches (chrome.settingsPrivate).
- Firefox saves through its login manager (Services.logins), and one profile
  gets a Primary Password the way Firefox's own dialog for it sets one
  (chrome://mozapps/content/preferences/changemp.js: the internal key
  token's changePassword), through Firefox's privileged scripts (Marionette's
  chrome context).

Firefox under automation sets preferences of its own for tests, password
saving turned off among them, so it starts here without those
(remote.prefs.recommended): the saving this reads is the profile's own.
`seed` ends with each browser saying its saving is on, so that `check`
finding it off is the import's doing.
"""

import json
import os
import pathlib
import sqlite3
import sys

from selenium import webdriver
from selenium.webdriver.chrome.service import Service as ChromeService
from selenium.webdriver.firefox.service import Service as FirefoxService

root = pathlib.Path(sys.argv[1]).resolve()
fakes = json.loads(pathlib.Path(sys.argv[2]).read_text())
step = sys.argv[3]
home = root / "home"
chrome_dir = home / ".config" / "google-chrome"
firefox_root = home / ".mozilla" / "firefox"
FIREFOX_PROFILES = {"plain.default-release": None, "primary.work": fakes["firefoxPrimary"]["password"]}


def chrome():
    options = webdriver.ChromeOptions()
    for arg in [
        "--headless=new",
        f"--user-data-dir={chrome_dir}",
        "--password-store=basic",
        "--no-first-run",
        "--no-default-browser-check",
    ]:
        options.add_argument(arg)
    service = ChromeService(os.path.join(os.environ["CHROMEWEBDRIVER"], "chromedriver"))
    return started(webdriver.Chrome(options=options, service=service))


def firefox(folder):
    profile = firefox_root / "Profiles" / folder
    profile.mkdir(parents=True, exist_ok=True)
    options = webdriver.FirefoxOptions()
    for arg in ["-headless", "-profile", str(profile)]:
        options.add_argument(arg)
    # Not automation's own preferences for tests, which turn password saving
    # off for as long as it runs: the profile's own, as the person has them.
    options.set_preference("remote.prefs.recommended", False)
    # Firefox's own privileged scripts (Marionette's chrome context), which
    # newer Firefox runs only when geckodriver asks for them.
    service = FirefoxService(
        os.path.join(os.environ["GECKOWEBDRIVER"], "geckodriver"),
        service_args=["--allow-system-access"],
    )
    return started(webdriver.Firefox(options=options, service=service))


def run(driver, script, *args):
    """Runs an async script; it calls done(null) or done(<what went wrong>)."""
    driver.set_script_timeout(60)
    problem = driver.execute_async_script(script, *args)
    if problem:
        name, version = driver.capabilities["browserName"], driver.capabilities["browserVersion"]
        raise SystemExit(
            f"{name} {version}: {problem}\n"
            "If the browser no longer has what this asks it for, see the top of "
            "tools/real-browsers/seed.py for which of its own pages to follow."
        )


def started(driver):
    print(f"Started {driver.capabilities['browserName']} {driver.capabilities['browserVersion']}", flush=True)
    return driver


SAVE_CHROME = """
const [logins, done] = arguments;
(async () => {
  for (const l of logins) {
    await chrome.passwordsPrivate.addPassword({
      url: l.url, username: l.username, password: l.password, note: l.note, useAccountStore: false,
    });
  }
  // The list catches up with the store on its own time.
  let saved = [];
  for (let i = 0; i < 100 && saved.length < logins.length; i++) {
    saved = await chrome.passwordsPrivate.getSavedPasswordList();
    if (saved.length < logins.length) await new Promise((r) => setTimeout(r, 200));
  }
  if (saved.length !== logins.length) {
    throw new Error(`Chrome has ${saved.length} saved (its API: ${Object.keys(chrome.passwordsPrivate).join(", ")})`);
  }
})().then(() => done(null), (e) => done(String(e && e.message || e)));
"""

# "Offer to save passwords and passkeys" and "Sign in automatically", as
# Chrome's settings page reads them.
CHROME_SAVING = """
const [on, done] = arguments;
Promise.all(["credentials_enable_service", "credentials_enable_autosignin"]
  .map((name) => chrome.settingsPrivate.getPref(name)))
  .then((prefs) => {
    const wrong = prefs.filter((p) => p.value !== on).map((p) => `${p.key} is ${p.value}`);
    done(wrong.length ? `its password saving should be ${on ? "on" : "off"}, but ${wrong.join(" and ")}` : null);
  }, (e) => done(String(e && e.message || e)));
"""

SAVE_FIREFOX = """
const [logins, primary, done] = arguments;
(async () => {
  for (const l of logins) {
    const login = Cc["@mozilla.org/login-manager/loginInfo;1"].createInstance(Ci.nsILoginInfo);
    login.init(l.url, l.url, null, l.username, l.password, "", "");
    await Services.logins.addLoginAsync(login);
  }
  const saved = (await Services.logins.getAllLogins()).length;
  if (saved !== logins.length) throw new Error(`it has ${saved} saved, not ${logins.length}`);
  if (primary) {
    // As Firefox's own Primary Password dialog sets one (changemp.js).
    const contract = "@mozilla.org/security/internalkeytoken;1";
    if (!Cc[contract] || !Ci.nsIPKCS11Token) {
      throw new Error(`it has no ${contract} (nsIPKCS11Token) to set a Primary Password with`);
    }
    const token = Cc[contract].createInstance(Ci.nsIPKCS11Token);
    if (token.hasPassword) throw new Error("its new profile already has a Primary Password");
    await token.changePassword("", primary);
    if (!token.hasPassword) throw new Error("setting a Primary Password left it without one");
  }
})().then(() => done(null), (e) => done(String(e && e.message || e)));
"""

# "Ask to save passwords" and "Fill usernames and passwords automatically",
# as Firefox itself has them.
FIREFOX_SAVING = """
const [on, done] = arguments;
try {
  const wrong = ["signon.rememberSignons", "signon.autofillForms"]
    .map((name) => [name, Services.prefs.getBoolPref(name)])
    .filter(([, value]) => value !== on)
    .map(([name, value]) => `${name} is ${value}`);
  done(wrong.length ? `its password saving should be ${on ? "on" : "off"}, but ${wrong.join(" and ")}` : null);
} catch (e) {
  done(String(e && e.message || e));
}
"""


def seed():
    chrome_dir.mkdir(parents=True, exist_ok=True)
    driver = chrome()
    try:
        driver.get("chrome://password-manager/passwords")
        run(driver, SAVE_CHROME, fakes["chrome"])
        driver.get("chrome://settings/passwords")
        run(driver, CHROME_SAVING, True)
    finally:
        driver.quit()
        for name in ["Login Data", "Login Data For Account"]:
            store = chrome_dir / "Default" / name
            if store.exists():
                rows = sqlite3.connect(f"file:{store}?mode=ro", uri=True).execute("SELECT count(*) FROM logins").fetchone()[0]
                print(f"Chrome's {name}: {rows} saved", flush=True)
    firefox_root.mkdir(parents=True, exist_ok=True)
    (firefox_root / "profiles.ini").write_text(
        "[Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/plain.default-release\nDefault=1\n\n"
        "[Profile1]\nName=work\nIsRelative=1\nPath=Profiles/primary.work\n\n"
        "[General]\nStartWithLastProfile=1\nVersion=2\n"
    )
    logins = {"plain.default-release": fakes["firefox"], "primary.work": fakes["firefoxPrimary"]["logins"]}
    for folder, primary in FIREFOX_PROFILES.items():
        driver = firefox(folder)
        try:
            with driver.context(driver.CONTEXT_CHROME):
                run(driver, SAVE_FIREFOX, logins[folder], primary)
                run(driver, FIREFOX_SAVING, True)
        finally:
            driver.quit()
        # What the automation left for itself, not the person's.
        (firefox_root / "Profiles" / folder / "user.js").unlink(missing_ok=True)


def check():
    driver = chrome()
    try:
        driver.get("chrome://settings/passwords")
        run(driver, CHROME_SAVING, False)
    finally:
        driver.quit()
    for folder in FIREFOX_PROFILES:
        driver = firefox(folder)
        try:
            with driver.context(driver.CONTEXT_CHROME):
                run(driver, FIREFOX_SAVING, False)
        finally:
            driver.quit()


{"seed": seed, "check": check}[step]()
print(f"{step}: done", flush=True)
