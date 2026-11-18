package com.passbundle.app

import android.Manifest
import android.app.Activity
import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import android.util.AtomicFile
import java.io.File
import java.io.IOException
import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId

/** One day's reminder: how many items expire soon and how many have expired, never which. */
data class ReminderDay(val date: LocalDate, val soon: Int, val expired: Int)

/**
 * The plain part of expiration reminders while PassBundle is closed ([Reminders]):
 * reading the days Dart hands over, when each is said, the words, and what's
 * kept between the app's runs. Nothing of Android's, so the JVM tests run it
 * (RemindersTest).
 */
object ReminderPlan {
    /** Each day's reminder is said at 9:00, where the phone is that day. */
    val AT: LocalTime = LocalTime.of(9, 0)

    /** The most days with a reminder waiting, as the core works them out. */
    const val MOST_DAYS = 30

    /** How long the days a reminder was said on are kept, for the app to see. */
    const val SAID_DAYS = 60L

    /**
     * What a reminder says, made here from its two numbers, as the core's
     * words (core/src/expiration.rs, `words`): how many items, never which.
     * Nothing else puts text in the notification. Null for none.
     */
    fun words(soon: Int, expired: Int): String? {
        val count = soon + expired
        if (soon < 0 || expired < 0 || count <= 0) return null
        val items = if (count == 1) "1 item" else "$count items"
        return when {
            soon == 0 -> "$items ${if (count == 1) "has expired" else "have expired"}"
            expired == 0 -> "$items ${if (count == 1) "expires soon" else "expire soon"}"
            else -> "$items expire soon or have expired"
        }
    }

    /**
     * The days Dart hands over (`{date, soon, expired}`, closed_reminders.dart):
     * only well-formed ones with something to say, each date once, soonest
     * first, at most [MOST_DAYS].
     */
    fun read(days: List<*>?): List<ReminderDay> =
        (days ?: emptyList<Any?>())
            .mapNotNull { day ->
                val map = day as? Map<*, *> ?: return@mapNotNull null
                val date = (map["date"] as? String)?.let(::date) ?: return@mapNotNull null
                val soon = (map["soon"] as? Number)?.toInt() ?: return@mapNotNull null
                val expired = (map["expired"] as? Number)?.toInt() ?: return@mapNotNull null
                ReminderDay(date, soon, expired).takeIf { words(soon, expired) != null }
            }.distinctBy { it.date }
            .sortedBy { it.date }
            .take(MOST_DAYS)

    /** "YYYY-MM-DD", exactly, or null. */
    fun date(text: String): LocalDate? =
        if (Regex("""\d{4}-\d{2}-\d{2}""").matches(text)) runCatching { LocalDate.parse(text) }.getOrNull() else null

    /** When [date]'s reminder is said, in [zone]. */
    fun at(date: LocalDate, zone: ZoneId): Instant = date.atTime(AT).atZone(zone).toInstant()

    /**
     * The days still to come after [now]: a day whose 9:00 has gone by (the
     * phone was off, or the plan came after it) isn't said late, and the app
     * says it when it opens, as it would have.
     */
    fun ahead(days: List<ReminderDay>, now: Instant, zone: ZoneId): List<ReminderDay> =
        days.filter { at(it.date, zone).isAfter(now) }

    /**
     * What the alarm for [date] says when it goes off on [today]: the plan's
     * day, if it's still in the plan (a later plan may have taken it away)
     * and it's still that day where the phone is. Otherwise nothing.
     */
    fun due(plan: List<ReminderDay>, date: LocalDate, today: LocalDate): ReminderDay? =
        plan.firstOrNull { it.date == date }?.takeIf { date == today }

    /** Each day's own number, for its alarm and its notification, so the next plan finds it. */
    fun number(date: LocalDate): Int = date.toEpochDay().toInt()

    /**
     * What's kept between the app's runs, in the app's no-backup folder: the
     * plan (dates and two numbers), the days a reminder was said, and whether
     * Android's question about notifications was asked.
     */
    data class Kept(
        val plan: List<ReminderDay> = emptyList(),
        val said: List<LocalDate> = emptyList(),
        val asked: Boolean = false,
    ) {
        fun encode(): String =
            buildString {
                if (asked) append("asked\n")
                for (day in plan) append("day ${day.date} ${day.soon} ${day.expired}\n")
                for (day in said) append("said $day\n")
            }

        /** Said on [date]: kept with the others of the last [SAID_DAYS] days. */
        fun saying(date: LocalDate): Kept =
            copy(said = (said + date).distinct().filter { !it.isBefore(date.minusDays(SAID_DAYS)) }.sorted())
    }

    /** What [Kept.encode] wrote; a line it can't read is left out. */
    fun decode(text: String): Kept {
        var asked = false
        val plan = mutableListOf<ReminderDay>()
        val said = mutableListOf<LocalDate>()
        for (line in text.lines()) {
            val parts = line.trim().split(' ')
            when (parts[0]) {
                "asked" -> asked = true
                "day" -> {
                    val date = parts.getOrNull(1)?.let(::date)
                    val soon = parts.getOrNull(2)?.toIntOrNull()
                    val expired = parts.getOrNull(3)?.toIntOrNull()
                    if (date != null && soon != null && expired != null && words(soon, expired) != null) {
                        plan += ReminderDay(date, soon, expired)
                    }
                }
                "said" -> parts.getOrNull(1)?.let(::date)?.let { said += it }
            }
        }
        return Kept(plan.distinctBy { it.date }.sortedBy { it.date }.take(MOST_DAYS), said.distinct().sorted(), asked)
    }
}

/**
 * Expiration reminders while PassBundle is closed (docs/phones.md, "Expiration
 * reminders while the app is closed"; closed_reminders.dart). Dart hands over a
 * plan: for each coming day with reminders, how many items expire soon and how
 * many have expired, never which. Each plan replaces the last: its alarms are
 * cancelled and the new days' set.
 *
 * AlarmManager, inexact (`setAndAllowWhileIdle`, `RTC_WAKEUP`): one alarm a
 * day at 9:00, which goes off in Doze too, with no exact-alarm permission (a
 * reminder a few minutes late is fine). WorkManager would be a library to add
 * and can't say "at 9:00"; JobScheduler can't either. Alarms don't survive a
 * restart, so [ReminderAlarm] sets the kept plan's again after one, after an
 * update, and when the clock or the time zone changes. A force stop clears
 * them until the app is opened again.
 *
 * When one goes off, the notification is made here, with words made from the
 * two numbers ([ReminderPlan.words]), and the day is kept as said, so the vault
 * doesn't say it again (Dart reads [said]). Tapping it opens the app, which
 * shows Expiring soon once it's unlocked ([openedBy], [takeOpened]).
 *
 * Android 13 and later ask the person about notifications once ([ask]); before
 * that, they're on unless the person turned them off.
 */
object Reminders {
    /** The notifications' channel, which the phone's settings show by name. */
    const val CHANNEL = "expiration-reminders"

    /** The alarm going off ([ReminderAlarm]). */
    const val FIRE = "com.passbundle.app.EXPIRATION_REMINDER"

    /** A reminder tapped (MainActivity). */
    const val OPEN = "com.passbundle.app.OPEN_EXPIRING_SOON"

    /** The day an alarm is for, "YYYY-MM-DD". */
    const val DATE = "date"

    private const val FILE = "expiration-reminders.txt"
    private const val TAG = "expiration-reminder"

    /** Any request code of the app's own: Android's question answers with it. */
    private const val ASK = 0x7e2

    @Volatile private var opened = false
    private var asking: ((String) -> Unit)? = null

    private fun file(context: Context) = AtomicFile(File(context.noBackupFilesDir, FILE))

    @Synchronized
    fun kept(context: Context): ReminderPlan.Kept =
        runCatching { ReminderPlan.decode(String(file(context).readFully(), Charsets.UTF_8)) }
            .getOrDefault(ReminderPlan.Kept())

    @Synchronized
    private fun keep(context: Context, kept: ReminderPlan.Kept) {
        val file = file(context)
        val out = runCatching { file.startWrite() }.getOrNull() ?: return
        try {
            out.write(kept.encode().toByteArray(Charsets.UTF_8))
            file.finishWrite(out)
        } catch (_: IOException) {
            file.failWrite(out)
        }
    }

    /** Puts [days] in place of the plan there was: how many alarms are set. */
    @Synchronized
    fun schedule(context: Context, days: List<ReminderDay>): Int {
        val app = context.applicationContext
        val was = kept(app)
        for (day in was.plan) cancel(app, day.date)
        keep(app, was.copy(plan = days))
        if (days.isNotEmpty()) channel(app)
        return set(app, days)
    }

    /** After a restart, an update, or the clock or time zone changing: the kept plan's alarms, again. */
    @Synchronized
    fun again(context: Context) {
        val app = context.applicationContext
        val plan = kept(app).plan
        for (day in plan) cancel(app, day.date)
        set(app, plan)
    }

    private fun set(context: Context, days: List<ReminderDay>): Int {
        val alarms = context.getSystemService(AlarmManager::class.java) ?: return 0
        val zone = ZoneId.systemDefault()
        val ahead = ReminderPlan.ahead(days, Instant.now(), zone)
        for (day in ahead) {
            alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, ReminderPlan.at(day.date, zone).toEpochMilli(), alarm(context, day.date))
        }
        return ahead.size
    }

    private fun alarm(context: Context, date: LocalDate): PendingIntent =
        PendingIntent.getBroadcast(
            context,
            ReminderPlan.number(date),
            Intent(context, ReminderAlarm::class.java).setAction(FIRE).putExtra(DATE, date.toString()),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

    private fun cancel(context: Context, date: LocalDate) {
        val alarm = alarm(context, date)
        context.getSystemService(AlarmManager::class.java)?.cancel(alarm)
        alarm.cancel()
    }

    /**
     * The alarm for [dateText] went off: its day's reminder, if the plan still
     * has it and it's still that day, while PassBundle's notifications are on.
     * Then the day is kept as said.
     */
    fun fire(context: Context, dateText: String?) {
        val app = context.applicationContext
        val date = dateText?.let(ReminderPlan::date) ?: return
        val kept = kept(app)
        val day = ReminderPlan.due(kept.plan, date, LocalDate.now()) ?: return
        val words = ReminderPlan.words(day.soon, day.expired) ?: return
        val manager = app.getSystemService(NotificationManager::class.java) ?: return
        if (access(app) != "allowed") return
        channel(app)
        val tap =
            PendingIntent.getActivity(
                app,
                ReminderPlan.number(date),
                Intent(app, MainActivity::class.java)
                    .setAction(OPEN)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        val notification =
            Notification.Builder(app, CHANNEL)
                .setSmallIcon(R.drawable.reminder_mark)
                .setContentTitle(words)
                .setContentIntent(tap)
                .setAutoCancel(true)
                .setCategory(Notification.CATEGORY_REMINDER)
                .build()
        try {
            manager.notify(TAG, ReminderPlan.number(date), notification)
        } catch (_: SecurityException) {
            return
        }
        keep(app, kept(app).saying(date))
    }

    /** The days a reminder was said on, the last 60: "YYYY-MM-DD". */
    fun said(context: Context): List<String> = kept(context.applicationContext).said.map { it.toString() }

    /**
     * Whether PassBundle's notifications show: "allowed"; "notAsked" on
     * Android 13 and later before Android has asked; else "refused" (the
     * person said no, or turned them, or this channel, off in the phone's
     * settings).
     */
    fun access(context: Context): String {
        val manager = context.getSystemService(NotificationManager::class.java)
        val channelOff = manager?.getNotificationChannel(CHANNEL)?.importance == NotificationManager.IMPORTANCE_NONE
        return when {
            manager?.areNotificationsEnabled() == true && !channelOff -> "allowed"
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                !kept(context.applicationContext).asked &&
                context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED -> "notAsked"
            else -> "refused"
        }
    }

    /**
     * Android's question about notifications, once (Android 13 and later):
     * [done] gets [access] once the person answers. Asked before, or before
     * Android 13, [done] gets it at once.
     */
    fun ask(activity: Activity, done: (String) -> Unit) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU || access(activity) != "notAsked") {
            done(access(activity))
            return
        }
        keep(activity.applicationContext, kept(activity.applicationContext).copy(asked = true))
        channel(activity.applicationContext)
        // An earlier question that never came back gets what's so now.
        asking?.invoke(access(activity))
        asking = done
        activity.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), ASK)
    }

    /** Android's answer to a question (MainActivity): true if it was this one's. */
    fun answered(context: Context, requestCode: Int): Boolean {
        if (requestCode != ASK) return false
        val done = asking ?: return true
        asking = null
        done(access(context))
        return true
    }

    /** The phone's settings for PassBundle's notifications: true if they opened. */
    fun openSettings(activity: Activity): Boolean =
        try {
            activity.startActivity(
                Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, activity.packageName),
            )
            true
        } catch (_: ActivityNotFoundException) {
            false
        }

    /** The app was opened, or brought back, by a tapped reminder: true if so. */
    fun openedBy(intent: Intent?): Boolean {
        if (intent?.action != OPEN) return false
        opened = true
        return true
    }

    /** Whether a reminder was tapped since this was last asked. */
    fun takeOpened(): Boolean {
        val was = opened
        opened = false
        return was
    }

    private fun channel(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        if (manager.getNotificationChannel(CHANNEL) != null) return
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Expiration reminders", NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = "How many items expire soon or have expired, never which."
            },
        )
    }
}

/**
 * A reminder's alarm going off ([Reminders.fire]), and the phone restarting,
 * the app being updated, or the clock or time zone changing, after which the
 * kept plan's alarms are set again ([Reminders.again]). Android sends these
 * to the app itself; nothing else can (not exported).
 */
class ReminderAlarm : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Reminders.FIRE -> Reminders.fire(context, intent.getStringExtra(Reminders.DATE))
            Intent.ACTION_BOOT_COMPLETED,
            Intent.ACTION_MY_PACKAGE_REPLACED,
            Intent.ACTION_TIME_CHANGED,
            Intent.ACTION_TIMEZONE_CHANGED,
            -> Reminders.again(context)
        }
    }
}
