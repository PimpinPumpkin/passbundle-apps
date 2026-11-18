package com.passbundle.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZonedDateTime

// Expiration reminders while PassBundle is closed (Reminders.kt): the days
// Dart hands over, read only when well formed; each said at 9:00 where the
// phone is, never late; the words, made here from two numbers as the core
// makes them; an alarm for a day a later plan took away saying nothing; and
// what's kept between runs.
class RemindersTest {
    private fun day(text: String) = LocalDate.parse(text)

    private fun handed(date: Any?, soon: Any?, expired: Any?) = mapOf("date" to date, "soon" to soon, "expired" to expired)

    @Test fun `a reminder says how many items, never which, in the core's words`() {
        assertNull(ReminderPlan.words(0, 0))
        assertEquals("1 item expires soon", ReminderPlan.words(1, 0))
        assertEquals("2 items expire soon", ReminderPlan.words(2, 0))
        assertEquals("1 item has expired", ReminderPlan.words(0, 1))
        assertEquals("2 items have expired", ReminderPlan.words(0, 2))
        assertEquals("3 items expire soon or have expired", ReminderPlan.words(2, 1))
        assertNull(ReminderPlan.words(-1, 2))
    }

    @Test fun `only well-formed days are read, each once, soonest first, at most 30`() {
        val days =
            ReminderPlan.read(
                listOf(
                    handed("2026-11-24", 2, 0),
                    handed("2026-11-13", 1, 0),
                    // The same day twice: the first.
                    handed("2026-11-13", 5, 5),
                    // Nothing to say, or not a day, or not numbers: left out.
                    handed("2026-11-14", 0, 0),
                    handed("2026-11-31", 1, 0),
                    handed("2026-1-5", 1, 0),
                    handed("Example Mail", 1, 0),
                    handed("2026-11-15", "1", 0),
                    handed("2026-11-16", 1, -1),
                    "2026-11-17",
                    null,
                ),
            )
        assertEquals(listOf(ReminderDay(day("2026-11-13"), 1, 0), ReminderDay(day("2026-11-24"), 2, 0)), days)
        assertEquals(emptyList<ReminderDay>(), ReminderPlan.read(null))
        val many = (0L until 40L).map { handed(day("2026-12-01").plusDays(it).toString(), 1, 0) }
        val read = ReminderPlan.read(many.reversed())
        assertEquals(30, read.size)
        assertEquals(day("2026-12-01"), read.first().date)
        assertEquals(day("2026-12-30"), read.last().date)
    }

    @Test fun `each day is said at 9 00 where the phone is, and a day whose 9 00 has gone by isn't said late`() {
        val chicago = ZoneId.of("America/Chicago")
        assertEquals(
            ZonedDateTime.of(2026, 11, 13, 9, 0, 0, 0, chicago).toInstant(),
            ReminderPlan.at(day("2026-11-13"), chicago),
        )
        // Across the change back from daylight time (November 1, 2026), still 9:00.
        assertEquals(Instant.parse("2026-11-02T15:00:00Z"), ReminderPlan.at(day("2026-11-02"), chicago))
        assertEquals(Instant.parse("2026-10-31T14:00:00Z"), ReminderPlan.at(day("2026-10-31"), chicago))
        val plan = listOf(ReminderDay(day("2026-11-12"), 1, 0), ReminderDay(day("2026-11-13"), 2, 0), ReminderDay(day("2026-11-20"), 0, 1))
        val before = ZonedDateTime.of(2026, 11, 13, 8, 59, 0, 0, chicago).toInstant()
        assertEquals(plan.drop(1), ReminderPlan.ahead(plan, before, chicago))
        val after = ZonedDateTime.of(2026, 11, 13, 9, 0, 0, 0, chicago).toInstant()
        assertEquals(plan.drop(2), ReminderPlan.ahead(plan, after, chicago))
    }

    @Test fun `an alarm says its day only while the plan has it, and only that day`() {
        val plan = listOf(ReminderDay(day("2026-11-13"), 2, 0))
        assertEquals(plan[0], ReminderPlan.due(plan, day("2026-11-13"), day("2026-11-13")))
        // A later plan took the day away (an item renewed, the setting off, signed out).
        assertNull(ReminderPlan.due(emptyList(), day("2026-11-13"), day("2026-11-13")))
        assertNull(ReminderPlan.due(plan, day("2026-11-14"), day("2026-11-14")))
        // Gone off after midnight: the app says it when it opens.
        assertNull(ReminderPlan.due(plan, day("2026-11-13"), day("2026-11-14")))
    }

    @Test fun `each day has its own number, for its alarm and its notification`() {
        assertEquals(20_770, ReminderPlan.number(day("2026-11-13")))
        assertTrue(ReminderPlan.number(day("2026-11-14")) != ReminderPlan.number(day("2026-11-13")))
    }

    @Test fun `what's kept is dates and numbers, read back as written, and the days said for 60 days`() {
        val kept =
            ReminderPlan.Kept(
                plan = listOf(ReminderDay(day("2026-11-13"), 2, 0), ReminderDay(day("2026-11-21"), 0, 2)),
                said = listOf(day("2026-11-01")),
                asked = true,
            )
        val text = kept.encode()
        assertEquals("asked\nday 2026-11-13 2 0\nday 2026-11-21 0 2\nsaid 2026-11-01\n", text)
        assertEquals(kept, ReminderPlan.decode(text))
        // Unreadable lines are left out; nothing kept is nothing.
        assertEquals(
            ReminderPlan.Kept(plan = listOf(ReminderDay(day("2026-11-13"), 1, 0))),
            ReminderPlan.decode("day 2026-11-13 1 0\nday tomorrow 1 0\nday 2026-11-14 0 0\nsaid soon\nhello\n"),
        )
        assertEquals(ReminderPlan.Kept(), ReminderPlan.decode(""))
        assertFalse(ReminderPlan.decode("").asked)

        val said = kept.saying(day("2026-11-13")).saying(day("2026-11-13"))
        assertEquals(listOf(day("2026-11-01"), day("2026-11-13")), said.said)
        assertEquals(kept.plan, said.plan)
        // Two months on, the oldest goes.
        assertEquals(listOf(day("2026-11-13"), day("2027-01-02")), said.saying(day("2027-01-02")).said)
    }
}
