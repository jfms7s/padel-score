import { afterEach, describe, expect, it, vi } from 'vitest';
import { addPlayer, getHistory, getPlayerPage, listPlayers, recordText, suggest, updatePlayer } from './players';
import type { Player } from './types';

const player = (id: string, name: string, archived = false): Player => ({
	id,
	name,
	archived,
	created_at: 1761400000,
	record: { wins: 0, losses: 0, draws: 0 }
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

	it('a_malformed_player_list_is_a_server_error', async () => {
		serve(200, { items: [{ id: 1 }] });
		await expect(listPlayers()).rejects.toMatchObject({ code: 'internal' });
	});
});
