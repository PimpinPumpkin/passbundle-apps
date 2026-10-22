import XCTest

/// PassBundle in iOS's AutoFill, the way someone uses it (docs/phones.md,
/// "Autofill" and "Passkeys"), driven by tools/ios-autofill-test.sh while the
/// app's half of the test waits signed in: turning PassBundle on in Settings,
/// filling a website's sign-in form in Safari from what iOS suggests above
/// the keyboard, then making a passkey on a website and signing in with it.
/// These drive Settings and Safari, not PassBundle's own app.
final class AutoFillUITests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  /// Settings, General, AutoFill & Passwords: PassBundle on.
  func testTurnOnInSettings() throws {
    let settings = XCUIApplication(bundleIdentifier: "com.apple.Preferences")
    settings.terminate()
    settings.launch()
    // Searching finds the page wherever this iOS keeps it.
    let search = settings.searchFields.firstMatch
    XCTAssertTrue(search.waitForExistence(timeout: 20), "Settings has no search field:\n\(settings.debugDescription)")
    search.tap()
    search.typeText("AutoFill")
    let page = settings.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'AutoFill'")).firstMatch
    XCTAssertTrue(page.waitForExistence(timeout: 20), "Settings didn't find AutoFill:\n\(settings.debugDescription)")
    page.tap()
    // AutoFill itself on, where this iOS has a switch for it, so the
    // providers show.
    let autoFill = settings.switches.matching(NSPredicate(format: "label BEGINSWITH 'AutoFill Passwords'")).firstMatch
    if autoFill.waitForExistence(timeout: 5), autoFill.value as? String != "1" {
      autoFill.tap()
    }
    let passBundle = settings.switches["PassBundle"]
    XCTAssertTrue(
      passBundle.waitForExistence(timeout: 20), "PassBundle isn't offered for AutoFill:\n\(settings.debugDescription)")
    if passBundle.value as? String != "1" {
      passBundle.tap()
    }
    XCTAssertEqual(passBundle.value as? String, "1")
  }

  /// Safari, already on the sign-in page: the email field offers
  /// PassBundle's login, picking it fills both fields, and signing in shows
  /// who signed in.
  func testFillInSafari() throws {
    let safari = XCUIApplication(bundleIdentifier: "com.apple.mobilesafari")
    safari.activate()
    let email = safari.webViews.textFields["Email"]
    XCTAssertTrue(email.waitForExistence(timeout: 30), "The sign-in page isn't open")
    email.tap()
    // Above the keyboard: whatever kind of element this iOS makes it.
    let suggestion = safari.descendants(matching: .any)
      .matching(
        NSPredicate(
          format:
            "label CONTAINS 'phone@example.test' AND elementType != \(XCUIElement.ElementType.textField.rawValue)"
        ))
      .firstMatch
    XCTAssertTrue(
      suggestion.waitForExistence(timeout: 20), "iOS didn't suggest PassBundle's login:\n\(safari.debugDescription)")
    suggestion.tap()
    // The extension opens for Face ID (debug builds' stand-in says yes), then fills.
    let signIn = safari.webViews.buttons["Sign in"]
    XCTAssertTrue(signIn.waitForExistence(timeout: 30))
    let filled = NSPredicate(format: "value == 'phone@example.test'")
    expectation(for: filled, evaluatedWith: email)
    waitForExpectations(timeout: 30)
    signIn.tap()
    let result = safari.webViews.staticTexts["Signed in as phone@example.test, password 25 characters"]
    XCTAssertTrue(result.waitForExistence(timeout: 20), "Signing in didn't show the filled login")
  }

  /// Safari, already on the passkeys page: making a passkey goes through
  /// iOS's sheet to PassBundle, which unlocks (debug builds' stand-in for
  /// Face ID says yes) and saves it once told to; signing in with it goes
  /// through iOS's sheet again, and the page checks the signature with the
  /// passkey's public key.
  func testPasskeysInSafari() throws {
    let safari = XCUIApplication(bundleIdentifier: "com.apple.mobilesafari")
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    safari.activate()
    let make = safari.webViews.buttons["Make a passkey"]
    XCTAssertTrue(make.waitForExistence(timeout: 30), "The passkeys page isn't open")
    make.tap()
    through(
      until: "Made a passkey for phone-passkey@example.test", in: safari, also: springboard,
      tapping: ["Save passkey", "Continue", "Save", "PassBundle"])
    safari.webViews.buttons["Sign in with a passkey"].tap()
    through(
      until: "Signed in with the passkey for phone-passkey@example.test", in: safari, also: springboard,
      tapping: ["Continue", "Sign In", "phone-passkey@example.test"])
  }

  /// Taps through iOS's sheets and PassBundle's screens until the page says
  /// `words`, whatever this iOS calls their buttons (the first of `labels`
  /// showing, outside the page). Fails with what the page said, or what was
  /// on screen.
  private func through(
    until words: String, in safari: XCUIApplication, also other: XCUIApplication, tapping labels: [String]
  ) {
    let done = safari.webViews.staticTexts[words]
    let failed = safari.webViews.staticTexts.matching(NSPredicate(format: "label BEGINSWITH \"Couldn't\"")).firstMatch
    let end = Date().addingTimeInterval(120)
    while Date() < end {
      if done.exists { return }
      if failed.exists { return XCTFail("The page said: \(failed.label)") }
      tapFirst(of: labels, in: [safari, other])
      Thread.sleep(forTimeInterval: 2)
    }
    XCTFail(
      "The page never said \"\(words)\".\nSafari:\n\(safari.debugDescription)\nSpringBoard:\n\(other.debugDescription)")
  }

  /// Taps the first button showing with one of the labels, never one of the page's own.
  private func tapFirst(of labels: [String], in apps: [XCUIApplication]) {
    for label in labels {
      for app in apps {
        let found = app.buttons.matching(NSPredicate(format: "label == %@", label)).allElementsBoundByIndex
        for button in found where button.isHittable && !app.webViews.buttons[label].exists {
          button.tap()
          return
        }
      }
    }
  }
}
