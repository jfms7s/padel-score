// Dates and times, shown in the device's own zone unless a zone is given (tests, and a board,
// which lays its week out in its own zone). One place for them, so every screen says
// "Wed 4 · 20:00–21:30" the same way. Instants are unix seconds. No library: the zone maths
// is `Intl.DateTimeFormat`. The service worker imports this file (through ./notice), so no DOM.

const DEFAULT_LOCALE = 'en-GB';
const DAY_SECS = 86_400;

const ms = (secs: number) => secs * 1000;

/** A short local date in the device's format ("04/11/2026"); `timeZone` for tests. */
export function dateText(secs: number, timeZone?: string): string {
	return new Date(ms(secs)).toLocaleDateString(undefined, timeZone === undefined ? undefined : { timeZone });
}

/** "Nov 2026". */
export function monthYear(secs: number, timeZone?: string): string {
	return new Date(ms(secs)).toLocaleDateString('en', { month: 'short', year: 'numeric', ...zone(timeZone) });
}

/** "Wed 4". */
export function dayLabel(secs: number, timeZone?: string, locale = DEFAULT_LOCALE): string {
	return new Date(ms(secs)).toLocaleDateString(locale, { weekday: 'short', day: 'numeric', ...zone(timeZone) });
}

/** "20:00". */
export function timeText(secs: number, timeZone?: string, locale = DEFAULT_LOCALE): string {
	return new Date(ms(secs)).toLocaleTimeString(locale, {
		hour: '2-digit',
		minute: '2-digit',
		hourCycle: 'h23',
		...zone(timeZone)
	});
}

/** "20:00–21:30". */
export function timeRange(start: number, end: number, timeZone?: string, locale = DEFAULT_LOCALE): string {
	return `${timeText(start, timeZone, locale)}–${timeText(end, timeZone, locale)}`;
}

const zone = (timeZone?: string) => (timeZone === undefined ? {} : { timeZone });

/** The device's IANA zone name, "UTC" when the browser won't say. */
export function browserTimeZone(): string {
	return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
}

const pad = (n: number) => String(n).padStart(2, '0');

interface Parts {
	year: number;
	month: number;
	day: number;
	hour: number;
	minute: number;
	second: number;
}

function partsIn(secs: number, timeZone: string): Parts {
	const f = new Intl.DateTimeFormat('en-US', {
		timeZone,
		hourCycle: 'h23',
		year: 'numeric',
		month: '2-digit',
		day: '2-digit',
		hour: '2-digit',
		minute: '2-digit',
		second: '2-digit'
	});
	const out: Record<string, number> = {};
	for (const p of f.formatToParts(new Date(ms(secs)))) if (p.type !== 'literal') out[p.type] = Number(p.value);
	return out as unknown as Parts;
}

/** The zone's offset from UTC at an instant, in seconds. */
function offsetAt(secs: number, timeZone: string): number {
	const p = partsIn(secs, timeZone);
	return Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute, p.second) / 1000 - Math.floor(secs);
}

/** The calendar day an instant falls on in the zone, "2026-11-04". */
export function localDate(secs: number, timeZone?: string): string {
	const p = partsIn(secs, timeZone ?? browserTimeZone());
	return `${p.year}-${pad(p.month)}-${pad(p.day)}`;
}

const utcDay = (date: string) => {
	const [y, m, d] = date.split('-').map(Number);
	return Date.UTC(y ?? 1970, (m ?? 1) - 1, d ?? 1) / 1000;
};

const isoDay = (utcSecs: number) => new Date(ms(utcSecs)).toISOString().slice(0, 10);

/** `date` plus `n` calendar days (negative for before). */
export function addDays(date: string, n: number): string {
	return isoDay(utcDay(date) + n * DAY_SECS);
}

/** The instant a local day starts, `date` as "2026-11-04". The offset is read twice, so a day
 *  whose clocks changed since the first guess still lands right. */
export function localMidnight(date: string, timeZone: string): number {
	const guess = utcDay(date);
	const first = guess - offsetAt(guess, timeZone);
	return guess - offsetAt(first, timeZone);
}

/** The Monday after `today` ("YYYY-MM-DD"): never today, so a Monday gives the next one. */
export function nextMonday(today: string): string {
	const weekday = new Date(ms(utcDay(today))).getUTCDay(); // 0 = Sunday
	return addDays(today, (8 - weekday) % 7 || 7);
}

export interface Day {
	date: string;
	start: number;
	end: number;
}

/** Every local day in the zone that touches `[startsAt, endsAt)`, each with its real length (a
 *  day can be 23, 24 or 25 hours long where the clocks change). A board made in another zone
 *  starts and ends part way through a local day, so a viewer can see 8 days; the cells outside
 *  the range are the grid's to switch off. */
export function weekDays(startsAt: number, endsAt: number, timeZone: string): Day[] {
	const first = localDate(startsAt, timeZone);
	const last = localDate(Math.max(startsAt, endsAt - 1), timeZone);
	const dates = [first];
	// A week is at most a few days over; the cap only guards a bad range.
	while (dates[dates.length - 1] !== last && dates.length < 14)
		dates.push(addDays(dates[dates.length - 1] ?? first, 1));
	const starts = [...dates, addDays(dates[dates.length - 1] ?? first, 1)].map((d) => localMidnight(d, timeZone));
	return dates.map((date, i) => ({ date, start: starts[i] ?? 0, end: starts[i + 1] ?? 0 }));
}
