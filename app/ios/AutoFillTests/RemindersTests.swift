import UserNotifications
import XCTest

/// Expiration reminders while PassBundle is closed (Runner/Reminders.swift,
/// compiled in here), with a stand-in for iOS's notification center, so
/// nothing is asked and nothing shows: the days Dart hands over, read only
/// when well formed; each at 9:00 on its day, never late; each plan in place
/// of the last, leaving anything not ours; the words, made from two numbers
/// as the core makes them; and the days said.
final class RemindersTests: XCTestCase {
  /// iOS's notification center, as far as Reminders uses it.
  final class Center: ReminderCenter {
    var status: UNAuthorizationStatus = .notDetermined
    var allow = true
    var asked = 0
    var pending: [UNNotificationRequest] = []
    var delivered: [String] = []

    func reminderAccess(_ done: @escaping (UNAuthorizationStatus) -> Void) { done(status) }

    func askForReminders(_ done: @escaping (Bool) -> Void) {
      asked += 1
      if status == .notDetermined { status = allow ? .authorized : .denied }
      done(status == .authorized)
    }

    func pendingIdentifiers(_ done: @escaping ([String]) -> Void) { done(pending.map(\.identifier)) }

    func deliveredIdentifiers(_ done: @escaping ([String]) -> Void) { done(delivered) }

    func removePending(_ identifiers: [String]) {
      pending.removeAll { identifiers.contains($0.identifier) }
    }

    func addReminder(_ request: UNNotificationRequest, _ done: @escaping (Bool) -> Void) {
      pending.append(request)
      done(true)
    }
  }

  /// A day as Dart hands it over: an NSDictionary of strings and numbers.
  private func handed(_ date: Any, _ soon: Any, _ expired: Any) -> [String: Any] {
    ["date": date, "soon": soon, "expired": expired]
  }

  private var chicago: Calendar {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "America/Chicago")!
    return calendar
  }

  private func at(_ text: String, _ hour: Int, _ minute: Int) -> Date {
    let (year, month, day) = Reminders.parts(text)!
    return chicago.date(from: DateComponents(year: year, month: month, day: day, hour: hour, minute: minute))!
  }

  /// Schedules and waits for iOS (the stand-in) to have them.
  private func schedule(_ days: [ReminderDay], _ center: Center, now: Date) -> Int {
    let done = expectation(description: "scheduled")
    var added = -1
    Reminders.schedule(days, center: center, now: now, calendar: chicago) {
      added = $0
      done.fulfill()
    }
    wait(for: [done], timeout: 5)
    return added
  }

  func testAReminderSaysHowManyNeverWhichInTheCoresWords() {
    XCTAssertNil(Reminders.words(soon: 0, expired: 0))
    XCTAssertEqual(Reminders.words(soon: 1, expired: 0), "1 item expires soon")
    XCTAssertEqual(Reminders.words(soon: 2, expired: 0), "2 items expire soon")
    XCTAssertEqual(Reminders.words(soon: 0, expired: 1), "1 item has expired")
    XCTAssertEqual(Reminders.words(soon: 0, expired: 2), "2 items have expired")
    XCTAssertEqual(Reminders.words(soon: 2, expired: 1), "3 items expire soon or have expired")
    XCTAssertNil(Reminders.words(soon: -1, expired: 2))
  }

  func testOnlyWellFormedDaysAreReadEachOnceSoonestFirstAtMostThirty() {
    let days = Reminders.read([
      handed("2026-11-24", 2, 0),
      handed("2026-11-13", 1, 0),
      handed("2026-11-13", 5, 5),
      handed("2026-11-14", 0, 0),
      handed("2026-11-31", 1, 0),
      handed("2026-1-5", 1, 0),
      handed("Example Mail", 1, 0),
      handed("2026-11-15", "1", 0),
      handed("2026-11-16", 1, -1),
      "2026-11-17",
      NSNull(),
    ] as [Any])
    XCTAssertEqual(days.map(\.date), ["2026-11-13", "2026-11-24"])
    XCTAssertEqual(days.map(\.soon), [1, 2])
    XCTAssertEqual(days[0].year, 2026)
    XCTAssertEqual(days[0].month, 11)
    XCTAssertEqual(days[0].day, 13)
    XCTAssertTrue(Reminders.read(nil).isEmpty)
    var many: [Any] = []
    for offset in (0..<40).reversed() {
      let date = chicago.date(byAdding: .day, value: offset, to: at("2026-12-01", 12, 0))!
      let parts = chicago.dateComponents([.year, .month, .day], from: date)
      many.append(handed(String(format: "%04d-%02d-%02d", parts.year!, parts.month!, parts.day!), 1, 0))
    }
    let read = Reminders.read(many)
    XCTAssertEqual(read.count, 30)
    XCTAssertEqual(read.first?.date, "2026-12-01")
    XCTAssertEqual(read.last?.date, "2026-12-30")
  }

  func testEachDayIsSaidAt9OnItsDayNeverLateAndEachPlanReplacesTheLast() {
    let center = Center()
    // Something of the app's own that isn't a reminder stays.
    let other = UNNotificationRequest(identifier: "something-else", content: UNNotificationContent(), trigger: nil)
    center.pending = [other]
    let days = Reminders.read([
      handed("2026-11-12", 1, 0), handed("2026-11-13", 2, 0), handed("2026-11-21", 0, 2),
    ])
    // The 13th at 8:59: the 12th is gone by, and isn't said late.
    XCTAssertEqual(schedule(days, center, now: at("2026-11-13", 8, 59)), 2)
    let ours = center.pending.filter { $0.identifier.hasPrefix(Reminders.prefix) }
    XCTAssertEqual(ours.map(\.identifier), ["expiring-2026-11-13", "expiring-2026-11-21"])
    let trigger = ours[0].trigger as? UNCalendarNotificationTrigger
    let when = trigger?.dateComponents
    XCTAssertEqual([when?.year, when?.month, when?.day, when?.hour, when?.minute], [2026, 11, 13, 9, 0])
    // No time zone: 9:00 wherever the phone is that day.
    XCTAssertNil(when?.timeZone)
    XCTAssertEqual(trigger?.repeats, false)
    XCTAssertEqual(ours.map(\.content.body), ["2 items expire soon", "2 items have expired"])
    // Only the words: nothing else in it, and no title.
    XCTAssertEqual(ours[0].content.title, "")
    XCTAssertEqual(ours[0].content.subtitle, "")
    XCTAssertTrue(ours[0].content.userInfo.isEmpty)

    // A new plan takes the old one's place, whole.
    let next = Reminders.read([handed("2026-11-20", 0, 1)])
    XCTAssertEqual(schedule(next, center, now: at("2026-11-13", 10, 0)), 1)
    XCTAssertEqual(center.pending.map(\.identifier), ["something-else", "expiring-2026-11-20"])
    // Signed out, or turned off: none of ours.
    XCTAssertEqual(schedule([], center, now: at("2026-11-13", 10, 0)), 0)
    XCTAssertEqual(center.pending.map(\.identifier), ["something-else"])
  }

  func testIOSIsAskedOnceAndItsAnswerIsReadAsTheVaultReadsIt() {
    XCTAssertEqual(Reminders.access(.notDetermined), "notAsked")
    XCTAssertEqual(Reminders.access(.authorized), "allowed")
    XCTAssertEqual(Reminders.access(.provisional), "allowed")
    XCTAssertEqual(Reminders.access(.denied), "refused")
    let center = Center()
    center.allow = false
    var answers: [String] = []
    Reminders.ask(center: center) { answers.append($0) }
    Reminders.ask(center: center) { answers.append($0) }
    XCTAssertEqual(answers, ["refused", "refused"])
    let allowing = Center()
    Reminders.ask(center: allowing) { answers.append($0) }
    Reminders.access(center: allowing) { answers.append($0) }
    XCTAssertEqual(answers.suffix(2), ["allowed", "allowed"])
  }

  func testTheDaysSaidAreThoseShownOrTappedAndThoseStillInNotificationCenterForSixtyDays() {
    let defaults = UserDefaults(suiteName: "RemindersTests-\(UUID().uuidString)")!
    let today = at("2026-11-20", 12, 0)
    Reminders.keepSaid("expiring-2026-11-13", defaults: defaults, today: today)
    Reminders.keepSaid("expiring-2026-11-13", defaults: defaults, today: today)
    // Not ours, and not a day: nothing kept.
    Reminders.keepSaid("something-else", defaults: defaults, today: today)
    Reminders.keepSaid("expiring-soon", defaults: defaults, today: today)
    let center = Center()
    center.delivered = ["expiring-2026-11-19", "something-else", "expiring-2026-08-01"]
    var said: [String] = []
    Reminders.said(center: center, defaults: defaults, today: today) { said = $0 }
    XCTAssertEqual(said, ["2026-11-13", "2026-11-19"])
    XCTAssertEqual(Reminders.recent(["2026-09-21", "2026-09-20", "2026-11-19"], today: today, calendar: chicago), [
      "2026-09-21", "2026-11-19",
    ])
  }
}
