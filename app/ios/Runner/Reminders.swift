import Foundation
import UserNotifications

/// One day's expiration reminder: how many items expire soon and how many
/// have expired, never which.
struct ReminderDay: Equatable {
  /// "YYYY-MM-DD".
  let date: String
  let year: Int
  let month: Int
  let day: Int
  let soon: Int
  let expired: Int
}

/// What `Reminders` needs of `UNUserNotificationCenter`, so the tests stand in
/// for it (AutoFillTests/RemindersTests.swift) without asking anyone anything.
protocol ReminderCenter: AnyObject {
  func reminderAccess(_ done: @escaping (UNAuthorizationStatus) -> Void)
  func askForReminders(_ done: @escaping (Bool) -> Void)
  func pendingIdentifiers(_ done: @escaping ([String]) -> Void)
  func deliveredIdentifiers(_ done: @escaping ([String]) -> Void)
  func removePending(_ identifiers: [String])
  func addReminder(_ request: UNNotificationRequest, _ done: @escaping (Bool) -> Void)
}

extension UNUserNotificationCenter: ReminderCenter {
  func reminderAccess(_ done: @escaping (UNAuthorizationStatus) -> Void) {
    getNotificationSettings { settings in done(settings.authorizationStatus) }
  }

  func askForReminders(_ done: @escaping (Bool) -> Void) {
    requestAuthorization(options: [.alert, .sound]) { granted, _ in done(granted) }
  }

  func pendingIdentifiers(_ done: @escaping ([String]) -> Void) {
    getPendingNotificationRequests { requests in done(requests.map(\.identifier)) }
  }

  func deliveredIdentifiers(_ done: @escaping ([String]) -> Void) {
    getDeliveredNotifications { notifications in done(notifications.map(\.request.identifier)) }
  }

  func removePending(_ identifiers: [String]) {
    removePendingNotificationRequests(withIdentifiers: identifiers)
  }

  func addReminder(_ request: UNNotificationRequest, _ done: @escaping (Bool) -> Void) {
    add(request) { error in done(error == nil) }
  }
}

/// Expiration reminders while PassBundle is closed (docs/phones.md,
/// "Expiration reminders while the app is closed"; closed_reminders.dart).
/// Dart hands over a plan: for each coming day with reminders, how many items
/// expire soon and how many have expired, never which. Each plan replaces the
/// last: every one of ours iOS has waiting goes, and the new days' are added,
/// each a calendar trigger at 9:00 on its day, in whatever time zone the
/// phone is in then. iOS keeps them across restarts.
///
/// The words are made here from the two numbers (`words`), so nothing else
/// puts text in a notification. A day iOS shows while the app is in front, a
/// tapped one, and any still in Notification Center count as said, so the
/// vault doesn't say them again (`said`); one swiped away before the app
/// opens can't be told from one never shown (the phone was off), and the
/// vault says it again. iOS asks the person about notifications once
/// (`ask`).
enum Reminders {
  /// Ours, among the app's notifications: "expiring-YYYY-MM-DD".
  static let prefix = "expiring-"

  /// Each day's is said at 9:00.
  static let hour = 9

  /// The most days with a reminder waiting, as the core works them out (iOS
  /// keeps 64 for an app).
  static let mostDays = 30

  /// How long the days said are kept, for the vault to see.
  static let saidDays = 60

  /// Where the days said are kept: the app's own defaults.
  static let saidKey = "expirationRemindersSaid"

  /// Tapped since the vault last asked.
  static var opened = false

  /// The core's words for the two numbers (core/src/expiration.rs, `words`):
  /// how many items, never which. Nil for none.
  static func words(soon: Int, expired: Int) -> String? {
    let count = soon + expired
    guard soon >= 0, expired >= 0, count > 0 else { return nil }
    let items = count == 1 ? "1 item" : "\(count) items"
    if soon == 0 { return "\(items) \(count == 1 ? "has expired" : "have expired")" }
    if expired == 0 { return "\(items) \(count == 1 ? "expires soon" : "expire soon")" }
    return "\(items) expire soon or have expired"
  }

  /// "YYYY-MM-DD", exactly, as a real day: its year, month, and day.
  static func parts(_ text: String) -> (Int, Int, Int)? {
    let pieces = text.split(separator: "-", omittingEmptySubsequences: false)
    guard text.count == 10, pieces.count == 3, pieces[0].count == 4, pieces[1].count == 2, pieces[2].count == 2,
      text.allSatisfy({ $0 == "-" || $0.isASCII && $0.isNumber }),
      let year = Int(pieces[0]), let month = Int(pieces[1]), let day = Int(pieces[2])
    else { return nil }
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "UTC")!
    let components = DateComponents(year: year, month: month, day: day)
    guard let date = calendar.date(from: components),
      calendar.dateComponents([.year, .month, .day], from: date) == components
    else { return nil }
    return (year, month, day)
  }

  /// The days Dart hands over (`{date, soon, expired}`): only well-formed
  /// ones with something to say, each date once, soonest first, at most 30.
  static func read(_ handed: Any?) -> [ReminderDay] {
    var seen = Set<String>()
    var days: [ReminderDay] = []
    for case let day as [String: Any] in (handed as? [Any]) ?? [] {
      guard let date = day["date"] as? String, let at = parts(date),
        let soon = day["soon"] as? Int, let expired = day["expired"] as? Int,
        words(soon: soon, expired: expired) != nil, seen.insert(date).inserted
      else { continue }
      days.append(ReminderDay(date: date, year: at.0, month: at.1, day: at.2, soon: soon, expired: expired))
    }
    return Array(days.sorted { $0.date < $1.date }.prefix(mostDays))
  }

  static func identifier(_ day: ReminderDay) -> String { prefix + day.date }

  /// The day one of ours is for, "YYYY-MM-DD"; nil for anything else.
  static func day(of identifier: String) -> String? {
    guard identifier.hasPrefix(prefix) else { return nil }
    let date = String(identifier.dropFirst(prefix.count))
    return parts(date) == nil ? nil : date
  }

  /// When a day's is said: 9:00 that day, wherever the phone is then.
  static func when(_ day: ReminderDay) -> DateComponents {
    DateComponents(year: day.year, month: day.month, day: day.day, hour: hour, minute: 0)
  }

  /// The days still to come after `now`: a day whose 9:00 has gone by isn't
  /// said late, and the vault says it when it opens, as it would have.
  static func ahead(_ days: [ReminderDay], now: Date, calendar: Calendar = .current) -> [ReminderDay] {
    days.filter { calendar.date(from: when($0)).map { $0 > now } ?? false }
  }

  /// A day's notification: its words, at 9:00.
  static func request(_ day: ReminderDay) -> UNNotificationRequest? {
    guard let words = words(soon: day.soon, expired: day.expired) else { return nil }
    let content = UNMutableNotificationContent()
    content.body = words
    content.sound = .default
    content.threadIdentifier = "expiration-reminders"
    let trigger = UNCalendarNotificationTrigger(dateMatching: when(day), repeats: false)
    return UNNotificationRequest(identifier: identifier(day), content: content, trigger: trigger)
  }

  /// Puts `days` in place of every one of ours iOS has waiting: `done(n)`,
  /// how many it took, once it has them.
  static func schedule(
    _ days: [ReminderDay], center: ReminderCenter, now: Date = Date(), calendar: Calendar = .current,
    done: @escaping (Int) -> Void
  ) {
    center.pendingIdentifiers { pending in
      center.removePending(pending.filter { $0.hasPrefix(prefix) })
      let requests = ahead(days, now: now, calendar: calendar).compactMap(request)
      guard !requests.isEmpty else { return done(0) }
      let group = DispatchGroup()
      let lock = NSLock()
      var added = 0
      for request in requests {
        group.enter()
        center.addReminder(request) { ok in
          lock.lock()
          if ok { added += 1 }
          lock.unlock()
          group.leave()
        }
      }
      group.notify(queue: .global()) { done(added) }
    }
  }

  /// "allowed", "refused", or "notAsked", as the vault reads them.
  static func access(_ status: UNAuthorizationStatus) -> String {
    switch status {
    case .authorized, .provisional: return "allowed"
    case .notDetermined: return "notAsked"
    default: return "refused"
    }
  }

  static func access(center: ReminderCenter, done: @escaping (String) -> Void) {
    center.reminderAccess { done(access($0)) }
  }

  /// iOS's question about notifications, once: `done` gets what's so after.
  /// Asked before, iOS answers at once.
  static func ask(center: ReminderCenter, done: @escaping (String) -> Void) {
    center.askForReminders { _ in access(center: center, done: done) }
  }

  /// A day of ours was said (shown in front, or tapped): kept, with the
  /// others of the last 60 days.
  static func keepSaid(_ identifier: String, defaults: UserDefaults = .standard, today: Date = Date()) {
    guard let date = day(of: identifier) else { return }
    let kept = (defaults.stringArray(forKey: saidKey) ?? []) + [date]
    defaults.set(recent(kept, today: today), forKey: saidKey)
  }

  /// The days said, the last 60: those kept, and those of ours still in
  /// Notification Center.
  static func said(
    center: ReminderCenter, defaults: UserDefaults = .standard, today: Date = Date(),
    done: @escaping ([String]) -> Void
  ) {
    center.deliveredIdentifiers { delivered in
      let all = (defaults.stringArray(forKey: saidKey) ?? []) + delivered.compactMap(day(of:))
      let kept = recent(all, today: today)
      defaults.set(kept, forKey: saidKey)
      done(kept)
    }
  }

  /// Each day once, sorted, from 60 days before `today` on.
  static func recent(_ days: [String], today: Date, calendar: Calendar = .current) -> [String] {
    let start = calendar.date(byAdding: .day, value: -saidDays, to: today) ?? today
    let parts = calendar.dateComponents([.year, .month, .day], from: start)
    let oldest = String(format: "%04d-%02d-%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0)
    return Array(Set(days.filter { $0 >= oldest })).sorted()
  }
}
