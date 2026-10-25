import { afterEach, describe, expect, it, vi } from 'vitest';
import { setUnauthorizedHandler } from './api';
import {
	addPlayer,
	getHistory,
	getPlayerPage,
	listPlayers,
	recordText,
	scoreText,
	suggest,
	updatePlayer
} from './players';
import { strings } from './strings';
import type { FinalScore, Player } from './types';

const player = (id: string, name: string, archived = false): Player => ({
	id,
	name,
	archived,
	created_at: 1761400000,
	record: { wins: 0, losses: 0, draws: 0 },
	link: null,
	pending_claims: 0
});

const ana = player('p-ana', 'Ana');
const rui = player('p-rui', 'Rui');
const marta = player('p-marta', 'Marta');
const joao = player('p-joao', 'João');
const old = player('p-old', 'Mariana', true);
const ALL = [ana, joao, marta, old, rui];

describe('suggest', () => {
	it('suggest_excludes_archived_and_taken', () => {
		const s = suggest(ALL, 'mar', ['p-marta']);
		expect(s.matches).toEqual([]);
		expect(suggest(ALL, 'ma', []).matches.map((p) => p.id)).toEqual(['p-marta']);
		expect(suggest(ALL, '', ['p-rui']).matches.map((p) => p.id)).toEqual(['p-ana', 'p-joao', 'p-marta']);
	});

	it('suggest_exact_match_is_case_and_space_insensitive', () => {
		const s = suggest(ALL, ' ANA ', []);
		expect(s.exact).toEqual(ana);
		expect(s.canAdd).toBe(false);
		expect(suggest(ALL, 'joão', []).exact).toEqual(joao);
	});

	it('suggest_can_add_new_name', () => {
		const s = suggest(ALL, 'Rita', []);
		expect(s).toEqual({ matches: [], exact: null, canAdd: true });
		// A part of a name is not the name: "An" can be added though it matches Ana.
		const part = suggest(ALL, 'An', []);
		expect(part.matches).toEqual([ana]);
		expect(part.exact).toBeNull();
		expect(part.canAdd).toBe(true);
		// An archived name can be added again, which restores it.
		expect(suggest(ALL, 'mariana', []).canAdd).toBe(true);
		expect(suggest(ALL, 'mariana', []).exact).toBeNull();
		// 40 characters at most, counted as characters.
		expect(suggest(ALL, 'ã'.repeat(40), []).canAdd).toBe(true);
		expect(suggest(ALL, 'x'.repeat(41), []).canAdd).toBe(false);
	});

	it('suggest_blank_cannot_add', () => {
		expect(suggest(ALL, '', []).canAdd).toBe(false);
		expect(suggest(ALL, '   ', []).canAdd).toBe(false);
		expect(suggest(ALL, '   ', []).exact).toBeNull();
	});

	it('suggest_caps_at_six', () => {
		const many = Array.from({ length: 9 }, (_, i) => player(`p${i}`, `Ana ${i}`));
		expect(suggest(many, 'ana', []).matches).toHaveLength(6);
		expect(suggest(many, '', []).matches).toHaveLength(6);
	});

	it('suggest_taken_name_cannot_add', () => {
		// Ana is picked in slot 1; slot 4 types "ana".
		const s = suggest(ALL, 'ana', ['p-ana']);
		expect(s.exact).toBeNull();
		expect(s.canAdd).toBe(false);
		expect(s.matches).toEqual([]);
	});

	it('suggest_puts_the_exact_match_first', () => {
		const anabela = player('p-anabela', 'Anabela');
		expect(suggest([anabela, ana], 'ana', []).matches.map((p) => p.id)).toEqual(['p-ana', 'p-anabela']);
	});
});

describe('recordText', () => {
	it('record_text_reads_wins_losses_draws', () => {
		expect(recordText({ wins: 3, losses: 1, draws: 0 })).toBe('3 W · 1 L · 0 D');
	});
});

const final = (over: Partial<FinalScore>): FinalScore => ({
	sets: [],
	games: [0, 0],
	regulation_games: [0, 0],
	overtime: false,
	timed: false,
	...over
});

describe('scoreText', () => {
	it('score_text_sets', () => {
		const score = final({
			sets: [
				{ a: 6, b: 4, tiebreak: null },
				{ a: 3, b: 6, tiebreak: null },
				{ a: 6, b: 2, tiebreak: null }
			],
			games: [6, 2],
			regulation_games: [6, 2]
		});
		expect(scoreText(score)).toBe('6-4 3-6 6-2');
		// A deciding super tie-break (games 1-0, the points in `tiebreak`) reads as its points.
		const stb = final({
			sets: [
				{ a: 6, b: 4, tiebreak: null },
				{ a: 3, b: 6, tiebreak: null },
				{ a: 1, b: 0, tiebreak: [10, 7] }
			],
			games: [1, 0],
			regulation_games: [1, 0]
		});
		expect(scoreText(stb)).toBe('6-4 3-6 10-7');
	});

	it('score_text_tiebreak_set', () => {
		// `tiebreak` is both teams' points [a, b]; the conventional figure is the loser's.
		const score = final({
			sets: [
				{ a: 7, b: 6, tiebreak: [7, 5] },
				{ a: 6, b: 7, tiebreak: [8, 10] }
			],
			games: [6, 7],
			regulation_games: [6, 7]
		});
		expect(scoreText(score)).toBe('7-6(5) 6-7(8)');
	});

	it('score_text_timed', () => {
		expect(scoreText(final({ timed: true, games: [7, 5], regulation_games: [7, 5] }))).toBe('7-5');
		expect(scoreText(final({ timed: true, games: [4, 4], regulation_games: [4, 4] }))).toBe('4-4');
	});

	it('score_text_overtime', () => {
		// The games at the end of regulation (the tie that sent it to overtime), then "overtime".
		const score = final({ timed: true, games: [6, 5], regulation_games: [5, 5], overtime: true });
		expect(scoreText(score)).toBe('5-5, overtime');
	});
});

describe('placing text', () => {
	it('placing_text_reads_shared_places', () => {
		expect(strings.history.placing(1, 1, 8, 'tournament')).toBe('1st of 8, tournament');
		expect(strings.history.placing(3, 4, 12, 'tournament')).toBe('3rd–4th of 12, tournament');
		expect(strings.history.placing(11, 12, 22, 'non_stop')).toBe('11th–12th of 22, non-stop');
		expect(strings.history.placing(21, 22, 22, 'non_stop')).toBe('21st–22nd of 22, non-stop');
	});
});

type Call = { url: string; init: RequestInit | undefined };

function serve(status: number, body: unknown): Call[] {
	const calls: Call[] = [];
	vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
		calls.push({ url, init });
		return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
	});
	return calls;
}

describe('players api', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('list_players_unwraps_items', async () => {
		const calls = serve(200, { items: [ana, old] });
		expect(await listPlayers()).toEqual([ana, old]);
		expect(calls[0]?.url).toBe('/api/players');
	});

	it('add_player_takes_created_or_existing', async () => {
		let calls = serve(201, ana);
		expect(await addPlayer('Ana')).toEqual(ana);
		expect(calls[0]?.init?.method).toBe('POST');
		expect(JSON.parse(String(calls[0]?.init?.body))).toEqual({ name: 'Ana' });
		calls = serve(200, ana);
		expect(await addPlayer('ana')).toEqual(ana);
	});

	it('update_player_patches_by_id', async () => {
		const calls = serve(200, { ...ana, archived: true });
		expect((await updatePlayer('p ana', { archived: true })).archived).toBe(true);
		expect(calls[0]?.url).toBe('/api/players/p%20ana');
		expect(calls[0]?.init?.method).toBe('PATCH');
		expect(JSON.parse(String(calls[0]?.init?.body))).toEqual({ archived: true });
	});

	it('history_and_player_page_pass_the_cursor', async () => {
		let calls = serve(200, { items: [], next: null });
		expect(await getHistory()).toEqual({ items: [], next: null });
		expect(calls[0]?.url).toBe('/api/me/history');
		calls = serve(200, { items: [], next: null });
		await getHistory('1761400000.a b');
		expect(calls[0]?.url).toBe('/api/me/history?before=1761400000.a%20b');
		const page = { player: ana, placings: [], items: [], next: null };
		calls = serve(200, page);
		expect(await getPlayerPage('p-ana', '1.x')).toEqual(page);
		expect(calls[0]?.url).toBe('/api/players/p-ana?before=1.x');
	});

	it('list_players_quiet_does_not_redirect_on_401', async () => {
		const redirect = vi.fn();
		const restore = setUnauthorizedHandler(redirect);
		try {
			serve(401, { error: 'unauthorized' });
			await expect(listPlayers({ quiet: true })).rejects.toMatchObject({ status: 401 });
			expect(redirect).not.toHaveBeenCalled();
			await expect(listPlayers()).rejects.toMatchObject({ status: 401 });
			expect(redirect).toHaveBeenCalledOnce();
		} finally {
			restore();
		}
	});

	it('a_malformed_player_list_is_a_server_error', async () => {
		serve(200, { items: [{ id: 1 }] });
		await expect(listPlayers()).rejects.toMatchObject({ code: 'internal' });
	});
});
