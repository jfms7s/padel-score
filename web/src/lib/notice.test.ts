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

	it('opens the calendar when the payload has no usable url', () => {
		expect(noticeText({ kind: 'test' }).url).toBe('/calendar');
		expect(noticeText({ kind: 'test', url: 'https://evil.test/x' }).tag).toBe('test:/calendar:');
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
			expect(safeNoticeUrl(bad)).toBe('/calendar');
		}
		expect(safeNoticeUrl('/b/abc')).toBe('/b/abc');
		expect(safeNoticeUrl('/j/abc?x=1')).toBe('/j/abc?x=1');
	});
});
