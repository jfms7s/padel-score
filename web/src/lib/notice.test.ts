import { describe, expect, it } from 'vitest';
import { noticeText, safeNoticeUrl } from './notice';

describe('noticeText', () => {
	it('says notifications are on for the test kind', () => {
		const n = noticeText({ kind: 'test', url: '/account' });
		expect(n.title).toBe('Notifications are on');
		expect(n.body).toBe("You'll get game and event updates on this device.");
		expect(n.url).toBe('/account');
		expect(n.tag).toBe('test:/account:');
	});

	it('falls back for a kind this build does not know', () => {
		const n = noticeText({ kind: 'from_the_future', url: '/b/abc' });
		expect(n.title).toBe('Padel score');
		expect(n.body).toBe('Something changed — tap to see');
		expect(n.url).toBe('/b/abc');
		expect(n.tag).toBe('from_the_future:/b/abc:');
	});

	it('opens Home when the payload has no usable url', () => {
		expect(noticeText({ kind: 'test' }).url).toBe('/');
		expect(noticeText({ kind: 'test', url: 'https://evil.test/x' }).tag).toBe('test:/:');
	});
});

describe('noticeText for the board kinds', () => {
	const game = {
		url: '/b/tok',
		title: 'Thursday padel',
		start: Date.UTC(2026, 10, 4, 20) / 1000,
		end: Date.UTC(2026, 10, 4, 21, 30) / 1000
	};
	const utc = { timeZone: 'UTC' };

	it('says a game formed', () => {
		const n = noticeText({ kind: 'game_formed', ...game }, utc);
		expect(n.title).toBe('Game formed');
		expect(n.body).toBe('Wed 4 · 20:00–21:30 · Thursday padel');
		expect(n.url).toBe('/b/tok');
		expect(n.tag).toBe(`game_formed:/b/tok:${game.start}`);
	});

	it('gives two games on one board different tags so one does not replace the other', () => {
		const a = noticeText({ kind: 'game_formed', ...game }, utc);
		const b = noticeText({ kind: 'game_formed', ...game, start: game.start + 86400 }, utc);
		expect(a.tag).not.toBe(b.tag);
	});

	it('says you moved into a game', () => {
		const n = noticeText({ kind: 'moved_in', ...game }, utc);
		expect(n.title).toBe("You're in a game");
		expect(n.body).toBe('Wed 4 · 20:00–21:30 · Thursday padel');
	});

	it('names who left', () => {
		const n = noticeText({ kind: 'player_left', name: 'Rui', ...game }, utc);
		expect(n.title).toBe('Rui left your game');
		expect(n.body).toBe('Wed 4 · 20:00–21:30 · Thursday padel');
	});

	it('says a board was deleted', () => {
		const n = noticeText({ kind: 'board_deleted', ...game, url: '/calendar' }, utc);
		expect(n.title).toBe('Board deleted');
		expect(n.body).toBe('Wed 4 · 20:00–21:30 · Thursday padel');
		expect(n.url).toBe('/calendar');
	});

	it('prints the time in the zone and locale it is given', () => {
		const tokyo = noticeText({ kind: 'game_formed', ...game }, { timeZone: 'Asia/Tokyo' });
		expect(tokyo.body).toBe('Thu 5 · 05:00–06:30 · Thursday padel');
		const lisbon = noticeText({ kind: 'game_formed', ...game }, { timeZone: 'Europe/Lisbon' });
		expect(lisbon.body).toBe('Wed 4 · 20:00–21:30 · Thursday padel');
		const fr = noticeText({ kind: 'game_formed', ...game }, { timeZone: 'UTC', locale: 'fr' });
		expect(fr.body).toBe('mer. 4 · 20:00–21:30 · Thursday padel');
	});

	it('leaves out what the payload lacks instead of printing undefined', () => {
		const n = noticeText({ kind: 'game_formed', url: '/b/tok' }, utc);
		expect(n.title).toBe('Game formed');
		expect(n.body).toBe('');
		expect(noticeText({ kind: 'player_left', url: '/b/tok' }, utc).title).toBe('Someone left your game');
	});
});

describe('noticeText for the game action kinds', () => {
	const base = {
		url: '/b/tok',
		title: 'Thursday padel',
		start: Date.UTC(2026, 10, 4, 20) / 1000,
		end: Date.UTC(2026, 10, 4, 21, 30) / 1000,
		new_start: Date.UTC(2026, 10, 4, 21) / 1000,
		new_end: Date.UTC(2026, 10, 4, 22, 30) / 1000
	};
	const utc = { timeZone: 'UTC' };
	const old = 'Wed 4 · 20:00–21:30 · Thursday padel';
	const moved = 'Wed 4 · 21:00–22:30 · Thursday padel';

	it('says someone gave you their place, with the game you are in now', () => {
		const n = noticeText({ kind: 'place_given', name: 'Rui', ...base }, utc);
		expect(n.title).toBe('Rui gave you their place');
		expect(n.body).toBe(moved);
		expect(n.url).toBe('/b/tok');
		expect(noticeText({ kind: 'place_given', url: '/b/tok' }, utc).title).toBe('Someone gave you their place');
	});

	it('says someone asks to swap, with your game', () => {
		const n = noticeText({ kind: 'swap_asked', name: 'Marta', ...base }, utc);
		expect(n.title).toBe('Marta asks to swap');
		expect(n.body).toBe(old);
	});

	it('says a swap was accepted or declined', () => {
		expect(noticeText({ kind: 'swap_answered', accepted: true, name: 'Rui', ...base }, utc).title).toBe(
			'Swap accepted'
		);
		const no = noticeText({ kind: 'swap_answered', accepted: false, name: 'Rui', ...base }, utc);
		expect(no.title).toBe('Swap declined');
		expect(no.body).toBe(old);
	});

	it('says someone proposes a new time, with the proposed one', () => {
		const n = noticeText({ kind: 'time_asked', name: 'Joana', ...base }, utc);
		expect(n.title).toBe('Joana proposes a new time');
		expect(n.body).toBe(moved);
	});

	it('says a new time was accepted or declined, with the current one', () => {
		expect(noticeText({ kind: 'time_answered', accepted: true, ...base }, utc).title).toBe('New time accepted');
		const no = noticeText({ kind: 'time_answered', accepted: false, name: 'Rui', ...base }, utc);
		expect(no.title).toBe('New time declined');
		expect(no.body).toBe(old);
	});

	it('says the game moved, with the new time', () => {
		const n = noticeText({ kind: 'time_changed', name: 'Rui', ...base }, utc);
		expect(n.title).toBe('Game moved');
		expect(n.body).toBe(moved);
	});

	it('prints the new time in the device zone', () => {
		const n = noticeText({ kind: 'time_changed', ...base }, { timeZone: 'Asia/Tokyo' });
		expect(n.body).toBe('Thu 5 · 06:00–07:30 · Thursday padel');
	});

	it('keeps the old game time when a payload lacks the new one', () => {
		expect(noticeText({ kind: 'time_changed', ...base, new_start: undefined, new_end: undefined }, utc).body).toBe(old);
	});
});

describe('safeNoticeUrl', () => {
	it('notice_url_must_be_a_same_origin_path', () => {
		for (const bad of [
			'https://evil.test/x',
			'//evil.test/x',
			'/\\evil.test',
			'javascript:alert(1)',
			42,
			undefined,
			null,
			'',
			'/\t/evil.test'
		]) {
			expect(safeNoticeUrl(bad)).toBe('/');
		}
		expect(safeNoticeUrl(undefined)).toBe('/');
		expect(safeNoticeUrl('/b/abc')).toBe('/b/abc');
		expect(safeNoticeUrl('/j/abc?x=1')).toBe('/j/abc?x=1');
	});
});

describe('noticeText for the event kinds', () => {
	const event = { url: '/j/tok', title: 'Sunday cup', start: Date.UTC(2026, 10, 4, 20) / 1000 };
	const utc = { timeZone: 'UTC' };
	const body = 'Wed 4 20:00 · Sunday cup';

	it.each([
		['event_cancelled', 'Event cancelled'],
		['event_changed', 'Event changed'],
		['team_complete', 'Your team is complete'],
		['moved_up', "You're in the event"],
		['moved_down', "You're on the waiting list"],
		['no_place', 'No place this time']
	])('says %s', (kind, title) => {
		const n = noticeText({ kind, ...event }, utc);
		expect(n.title).toBe(title);
		expect(n.body).toBe(body);
		expect(n.url).toBe('/j/tok');
		expect(n.tag).toBe(`${kind}:/j/tok:${event.start}`);
	});

	it('names who left the team', () => {
		const n = noticeText({ kind: 'partner_left', name: 'Rui', ...event }, utc);
		expect(n.title).toBe('Rui left your team');
		expect(n.body).toBe(body);
		expect(noticeText({ kind: 'partner_left', ...event }, utc).title).toBe('Someone left your team');
	});

	it('prints the start in the device zone and leaves out what is missing', () => {
		expect(noticeText({ kind: 'event_changed', ...event }, { timeZone: 'Asia/Tokyo' }).body).toBe(
			'Thu 5 05:00 · Sunday cup'
		);
		expect(noticeText({ kind: 'event_cancelled', url: '/j/tok', title: 'Sunday cup' }, utc).body).toBe('Sunday cup');
		expect(noticeText({ kind: 'moved_up', url: '/j/tok' }, utc).body).toBe('');
	});
});

describe('noticeText for the scheduled match kinds', () => {
	const match = { url: '/j/tok', name: 'Ana', start: Date.UTC(2026, 10, 4, 20) / 1000 };
	const utc = { timeZone: 'UTC' };

	it.each([
		['place_opened', 'You got a place'],
		['match_changed', 'Match changed'],
		['match_cancelled', 'Match cancelled'],
		['removed_from_match', 'Removed from the match'],
		['match_short', 'One player missing']
	])('says %s', (kind, title) => {
		const n = noticeText({ kind, ...match }, utc);
		expect(n.title).toBe(title);
		expect(n.body).toBe('Wed 4 20:00 · Ana');
		expect(n.url).toBe('/j/tok');
		expect(n.tag).toBe(`${kind}:/j/tok:${match.start}`);
	});

	it('leaves out what the payload lacks', () => {
		expect(noticeText({ kind: 'match_cancelled', url: '/j/tok' }, utc).body).toBe('');
	});
});

describe('noticeText for the reminders', () => {
	const utc = { timeZone: 'UTC' };
	const start = Date.UTC(2026, 10, 4, 20) / 1000;

	it('says a game is one player short, with its time', () => {
		const n = noticeText({ kind: 'short_game', url: '/b/tok', title: 'Thursday padel', start, end: start + 5400 }, utc);
		expect(n.title).toBe('One player missing');
		expect(n.body).toBe('Wed 4 · 20:00–21:30 · Thursday padel');
		expect(n.url).toBe('/b/tok');
		expect(n.tag).toBe(`short_game:/b/tok:${start}`);
	});

	it('says an event starts in 24 hours, in the device zone', () => {
		const n = noticeText({ kind: 'event_soon', url: '/j/tok', title: 'Sunday cup', start }, utc);
		expect(n.title).toBe('Starts in 24 hours');
		expect(n.body).toBe('Wed 4 20:00 · Sunday cup');
		expect(n.url).toBe('/j/tok');
		const tokyo = noticeText(
			{ kind: 'event_soon', url: '/j/tok', title: 'Sunday cup', start },
			{ timeZone: 'Asia/Tokyo' }
		);
		expect(tokyo.body).toBe('Thu 5 05:00 · Sunday cup');
	});

	it('keeps the same-origin guard first for the new kinds', () => {
		expect(noticeText({ kind: 'short_game', url: '//evil.test/x' }, utc).url).toBe('/');
		expect(noticeText({ kind: 'event_soon', url: 'https://evil.test' }, utc).url).toBe('/');
	});

	it('leaves out what the payload lacks', () => {
		expect(noticeText({ kind: 'short_game', url: '/b/tok' }, utc).body).toBe('');
		expect(noticeText({ kind: 'event_soon', url: '/j/tok' }, utc).body).toBe('');
	});
});
