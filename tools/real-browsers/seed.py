"""Real Chrome and Firefox, saving made-up passwords (fakes.json) in profiles
in a throwaway home folder, for the import's test against real browsers
(browsers/tests/real_browsers.rs). CI only, on a fresh Linux runner: it never
looks at anyone's own profiles.

    seed.py <root> fakes.json seed     # makes <root>/home and fills it
    seed.py <root> fakes.json check    # after the import turned saving off

Chrome saves through its own password manager page's API
(chrome://password-manager, chrome.passwordsPrivate), with its basic store
(no keyring), as a Linux desktop without one has it. Firefox saves through
its login manager, and one profile gets a Primary Password, through Firefox's
own privileged scripts (Marionette's chrome context).
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
    return webdriver.Chrome(options=options, service=service)


def firefox(folder):
    profile = firefox_root / "Profiles" / folder
    profile.mkdir(parents=True, exist_ok=True)
    options = webdriver.FirefoxOptions()
    for arg in ["-headless", "-profile", str(profile), "-remote-allow-system-access"]:
        options.add_argument(arg)
    service = FirefoxService(os.path.join(os.environ["GECKOWEBDRIVER"], "geckodriver"))
    return webdriver.Firefox(options=options, service=service)


def run(driver, script, *args):
    """Runs an async script; it calls done(null) or done(<what went wrong>)."""
    driver.set_script_timeout(60)
    problem = driver.execute_async_script(script, *args)
    if problem:
        raise SystemExit(problem)


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

CHROME_SAVING = """
const done = arguments[0];
chrome.settingsPrivate.getPref("credentials_enable_service")
  .then((p) => done(p.value === false ? null : `Chrome's saving is ${p.value}`),
        (e) => done(String(e && e.message || e)));
"""

SAVE_FIREFOX = """
const [logins, primary, done] = arguments;
(async () => {
  for (const l of logins) {
    const login = Cc["@mozilla.org/login-manager/loginInfo;1"].createInstance(Ci.nsILoginInfo);
    login.init(l.url, l.url, null, l.username, l.password, "", "");
    await Services.logins.addLoginAsync(login);
  }
  if (primary) {
    const token = Cc["@mozilla.org/pk11tokendb;1"].getService(Ci.nsIPK11TokenDB).getInternalKeyToken();
    if (token.needsUserInit) token.initPassword(primary);
    else token.changePassword("", primary);
  }
})().then(() => done(null), (e) => done(String(e && e.message || e)));
"""

FIREFOX_SAVING = """
const done = arguments[0];
const saving = Services.prefs.getBoolPref("signon.rememberSignons", true);
done(saving ? "Firefox's saving is on" : null);
"""


def seed():
    chrome_dir.mkdir(parents=True, exist_ok=True)
    driver = chrome()
    try:
        driver.get("chrome://password-manager/passwords")
        run(driver, SAVE_CHROME, fakes["chrome"])
    finally:
        driver.quit()
        for name in ["Login Data", "Login Data For Account"]:
            store = chrome_dir / "Default" / name
            if store.exists():
                rows = sqlite3.connect(f"file:{store}?mode=ro", uri=True).execute("SELECT count(*) FROM logins").fetchone()[0]
                print(f"Chrome's {name}: {rows} saved")
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
        finally:
            driver.quit()
        # What the automation left for itself, not the person's.
        (firefox_root / "Profiles" / folder / "user.js").unlink(missing_ok=True)


def check():
    driver = chrome()
    try:
        driver.get("chrome://settings/passwords")
        run(driver, CHROME_SAVING)
    finally:
        driver.quit()
    for folder in FIREFOX_PROFILES:
        driver = firefox(folder)
        try:
            with driver.context(driver.CONTEXT_CHROME):
                run(driver, FIREFOX_SAVING)
        finally:
            driver.quit()


{"seed": seed, "check": check}[step]()
print(f"{step}: done")
