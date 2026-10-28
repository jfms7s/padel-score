import { describe, expect, it } from 'vitest';
import {
	dateText,
	dayLabel,
	localDate,
	localMidnight,
	monthYear,
	nextMonday,
	timeRange,
	timeText,
	weekDays
} from './dates';

// 2026-11-04 20:00 UTC, a Wednesday.
const WED_2000 = Date.UTC(2026, 10, 4, 20, 0) / 1000;

describe('dates', () => {
	it('labels a day and a time in the given zone', () => {
		expect(dayLabel(WED_2000, 'UTC')).toBe('Wed 4');
		expect(timeText(WED_2000, 'UTC')).toBe('20:00');
		// Tokyo is nine hours ahead: already Thursday the 5th.
		expect(dayLabel(WED_2000, 'Asia/Tokyo')).toBe('Thu 5');
		expect(timeText(WED_2000, 'Asia/Tokyo')).toBe('05:00');
	});

	it('writes midnight as 00:00, not 24:00', () => {
		expect(timeText(Date.UTC(2026, 10, 4, 0, 0) / 1000, 'UTC')).toBe('00:00');
	});

	it('timeRange joins two times with an en dash', () => {
		expect(timeRange(WED_2000, WED_2000 + 5400, 'UTC')).toBe('20:00–21:30');
	});

	it('dateText keeps the device format and honours a zone', () => {
		expect(dateText(WED_2000, 'UTC')).toBe(
			new Date(WED_2000 * 1000).toLocaleDateString(undefined, { timeZone: 'UTC' })
		);
		expect(dateText(WED_2000)).toBe(new Date(WED_2000 * 1000).toLocaleDateString());
	});

	it('monthYear is a short month and the year', () => {
		expect(monthYear(WED_2000, 'UTC')).toBe('Nov 2026');
	});

	it('localDate is the calendar day in the zone', () => {
		expect(localDate(WED_2000, 'UTC')).toBe('2026-11-04');
		expect(localDate(WED_2000, 'Asia/Tokyo')).toBe('2026-11-05');
	});

	it('localMidnight finds the start of a local day', () => {
		expect(localMidnight('2026-11-04', 'UTC')).toBe(Date.UTC(2026, 10, 4) / 1000);
		// Lisbon is on UTC+1 in summer time, UTC+0 after the end of October.
		expect(localMidnight('2026-10-19', 'Europe/Lisbon')).toBe(Date.UTC(2026, 9, 18, 23) / 1000);
		expect(localMidnight('2026-10-26', 'Europe/Lisbon')).toBe(Date.UTC(2026, 9, 26) / 1000);
		expect(localMidnight('2026-11-05', 'Asia/Tokyo')).toBe(Date.UTC(2026, 10, 4, 15) / 1000);
	});

	it('localMidnight is the first instant of the day where the clocks jump at midnight', () => {
		// Each zone has no 00:00 on the given day: the day starts at 01:00.
		for (const [date, zone] of [
			['2027-03-28', 'Atlantic/Azores'],
			['2026-04-24', 'Africa/Cairo'],
			['2026-09-06', 'America/Santiago']
		] as const) {
			const m = localMidnight(date, zone);
			expect(localDate(m, zone)).toBe(date);
			expect(localDate(m - 1, zone)).not.toBe(date);
		}
		// Azores: 01:00 UTC is when the clocks go from 00:00 (UTC-1) to 01:00 (UTC+0).
		expect(localMidnight('2027-03-28', 'Atlantic/Azores')).toBe(Date.UTC(2027, 2, 28, 1) / 1000);
	});

	it('a week across a midnight spring-forward has a 23-hour Sunday and no lost Saturday hour', () => {
		const tz = 'Atlantic/Azores';
		const days = weekDays(localMidnight('2027-03-22', tz), localMidnight('2027-03-29', tz), tz);
		expect(days.map((d) => d.end - d.start)).toEqual([86400, 86400, 86400, 86400, 86400, 86400, 82800]);
		expect(new Set(days.map((d) => dayLabel(d.start, tz))).size).toBe(7);
	});

	it('nextMonday is the Monday after today, never today', () => {
		expect(nextMonday('2026-10-26')).toBe('2026-11-02');
		expect(nextMonday('2026-10-25')).toBe('2026-10-26');
		expect(nextMonday('2026-10-28')).toBe('2026-11-02');
		expect(nextMonday('2026-12-28')).toBe('2027-01-04');
	});

	it('week_across_dst_end_has_a_25_hour_sunday', () => {
		const tz = 'Europe/Lisbon';
		const days = weekDays(localMidnight('2026-10-19', tz), localMidnight('2026-10-26', tz), tz);
		expect(days).toHaveLength(7);
		expect(days.map((d) => d.date)).toEqual([
			'2026-10-19',
			'2026-10-20',
			'2026-10-21',
			'2026-10-22',
			'2026-10-23',
			'2026-10-24',
			'2026-10-25'
		]);
		const lengths = days.map((d) => d.end - d.start);
		expect(lengths[6]).toBe(90000);
		expect(lengths.slice(0, 6)).toEqual([86400, 86400, 86400, 86400, 86400, 86400]);
		// The days follow each other with no gap.
		for (let i = 1; i < 7; i++) expect(days[i]?.start).toBe(days[i - 1]?.end);
	});

	it('a viewer in another zone sees the 8 local days a Lisbon week touches', () => {
		// The board runs Lisbon Mon 2 Nov 00:00 to Mon 9 Nov 00:00 (UTC+0). New York is UTC-5 (its
		// clocks went back on Sunday 1 Nov), so the week starts on Sunday evening there.
		const starts = localMidnight('2026-11-02', 'Europe/Lisbon');
		const ends = localMidnight('2026-11-09', 'Europe/Lisbon');
		const days = weekDays(starts, ends, 'America/New_York');
		expect(days.map((d) => d.date)).toEqual([
			'2026-11-01',
			'2026-11-02',
			'2026-11-03',
			'2026-11-04',
			'2026-11-05',
			'2026-11-06',
			'2026-11-07',
			'2026-11-08'
		]);
		expect(days[0]!.end - days[0]!.start).toBe(90000);
		expect(days[0]!.start).toBeLessThan(starts);
		expect(days[0]!.end).toBeGreaterThan(starts);
		expect(days[7]!.end).toBeGreaterThan(ends);
		expect(days[7]!.start).toBeLessThan(ends);
	});

	it('a week across the DST start has a 23-hour Sunday', () => {
		const tz = 'Europe/Lisbon';
		const days = weekDays(localMidnight('2026-03-23', tz), localMidnight('2026-03-30', tz), tz);
		expect(days.map((d) => d.end - d.start)).toEqual([86400, 86400, 86400, 86400, 86400, 86400, 82800]);
	});
});
