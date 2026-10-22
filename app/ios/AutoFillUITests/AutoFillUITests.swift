import XCTest

/// PassBundle in iOS's AutoFill, the way someone uses it (docs/phones.md,
/// "Autofill"), driven by tools/ios-autofill-test.sh while the app's half of
/// the test waits signed in: turning PassBundle on in Settings, then filling
/// a website's sign-in form in Safari from what iOS suggests above the
/// keyboard. These drive Settings and Safari, not PassBundle's own app.
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
    XCTAssertTrue(search.waitForExistence(timeout: 20), "Settings has no search field")
    search.tap()
    search.typeText("AutoFill")
    let page = settings.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'AutoFill'")).firstMatch
    XCTAssertTrue(page.waitForExistence(timeout: 20), "Settings didn't find AutoFill")
    page.tap()
    // AutoFill itself on, where this iOS has a switch for it, so the
    // providers show.
    let autoFill = settings.switches.matching(NSPredicate(format: "label BEGINSWITH 'AutoFill Passwords'")).firstMatch
    if autoFill.waitForExistence(timeout: 5), autoFill.value as? String != "1" {
      autoFill.tap()
    }
    let passBundle = settings.switches["PassBundle"]
    XCTAssertTrue(passBundle.waitForExistence(timeout: 20), "PassBundle isn't offered for AutoFill")
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
    let suggestion = safari.buttons.matching(NSPredicate(format: "label CONTAINS 'phone@example.test'")).firstMatch
    XCTAssertTrue(suggestion.waitForExistence(timeout: 20), "iOS didn't suggest PassBundle's login")
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
}
