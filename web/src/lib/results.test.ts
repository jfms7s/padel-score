import { describe, expect, it } from 'vitest';
import { longDate } from './dates';
import historyPage from './fixtures/history-page.json';
import { playedRow, recordedRow, showFindYourself, viewFromQuery } from './results';
import { strings } from './strings';
import type { HistoryEntry, LinkedEntry } from './types';

const recorded = historyPage.items[0] as unknown as HistoryEntry;
const score = recorded.score;

const played = (over: Partial<Extract<LinkedEntry, { redacted: false }>> = {}): LinkedEntry => ({
	match_id: 'm1',
	finished_at: 1761400000,
	outcome: 'win',
	side: 'a',
	score,
	redacted: false,
	teams: { a: ['Ana', 'Marta'], b: ['Rui', 'João'] },
	coordinator_name: 'Coach',
	spectator_token: 'v-abc',
	...over
});

describe('viewFromQuery', () => {
	it('is recorded only for ?view=recorded', () => {
		expect(viewFromQuery(new URLSearchParams('view=recorded'))).toBe('recorded');
	});
	it('is played with no view or another one', () => {
		expect(viewFromQuery(new URLSearchParams(''))).toBe('played');
		expect(viewFromQuery(new URLSearchParams('view=x'))).toBe('played');
	});
});

describe('recordedRow', () => {
	it('names both teams, links the final view and says recorded', () => {
		const row = recordedRow(recorded, 'UTC');
		expect(row.a).toBe('Ana / Marta');
		expect(row.b).toBe('Rui / João');
		expect(row.winner).toBe('a');
		expect(row.chip).toBe('recorded');
		expect(row.href).toBe('/v/' + recorded.spectator_token);
		expect(row.date).toBe(longDate(recorded.finished_at, 'UTC'));
		expect(row.event).toBeNull();
		expect(row.datetime).toBe(new Date(recorded.finished_at * 1000).toISOString());
	});
	it('keeps the names and picked player ids for linking', () => {
		const row = recordedRow(recorded);
		expect(row.teams).toEqual(recorded.teams);
		expect(row.players).toEqual(recorded.players);
	});
	it('carries the event label', () => {
		const event = { id: 'e1', name: 'Cup', kind: 'tournament' as const, label: 'Semi-final' };
		expect(recordedRow({ ...recorded, event }).event).toBe('Semi-final');
	});
});

describe('playedRow', () => {
	it('maps the outcome to a chip and the winning side', () => {
		const win = playedRow(played({ outcome: 'win', side: 'b' }));
		expect([win.chip, win.winner]).toEqual(['won', 'b']);
		const loss = playedRow(played({ outcome: 'loss', side: 'b' }));
		expect([loss.chip, loss.winner]).toEqual(['lost', 'a']);
		const draw = playedRow(played({ outcome: 'draw' }));
		expect([draw.chip, draw.winner]).toEqual(['draw', null]);
	});
	it('names the teams, links the final view and shows the score', () => {
		const row = playedRow(played());
		expect([row.a, row.b]).toEqual(['Ana / Marta', 'Rui / João']);
		expect(row.href).toBe('/v/v-abc');
		expect(row.teams.a).toEqual(['Ana', 'Marta']);
		expect(row.players).toBeNull();
		expect(row.score).toBe(recordedRow(recorded).score);
	});
	it('hides the names and the link on a redacted entry', () => {
		const row = playedRow({
			match_id: 'm2',
			finished_at: 1761400000,
			outcome: 'loss',
			side: 'a',
			score,
			redacted: true
		});
		expect([row.a, row.b]).toEqual([strings.results.hidden, strings.results.hidden]);
		expect(row.href).toBeNull();
		expect(row.players).toBeNull();
	});
	it('falls back to the event kind on a redacted event', () => {
		const row = playedRow({
			match_id: 'm3',
			finished_at: 1761400000,
			outcome: 'win',
			side: 'a',
			score,
			redacted: true,
			event: { kind: 'tournament' }
		});
		expect(row.event).toBe(strings.events.kinds.tournament);
	});
});

describe('showFindYourself', () => {
	it('shows only with recorded matches and none played', () => {
		expect(showFindYourself(0, 3)).toBe(true);
		expect(showFindYourself(0, 0)).toBe(false);
		expect(showFindYourself(2, 3)).toBe(false);
	});
});
