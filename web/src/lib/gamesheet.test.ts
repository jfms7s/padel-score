import { describe, expect, it } from 'vitest';
import { gameActions, requestCards, swapTargets, timeOptions, type GameAction } from './gamesheet';
import type { BoardView, GameOut, PieceOut, RequestOut } from './types';

const H = 3600;
const T0 = Date.UTC(2026, 10, 2) / 1000; // Monday 2026-11-02 00:00 UTC
/** Wednesday 20:00 UTC, the game most tests look at. */
const G = T0 + 2 * 86400 + 20 * H;

const person = (id: string) => ({ user_id: id, name: id.toUpperCase() });
const piece = (user_id: string, start: number, end: number, created_at = 0): PieceOut => ({
	user_id,
	start,
	end,
	created_at
});

function game(id: string, start: number, players: string[], over: Partial<GameOut> = {}): GameOut {
	return {
		id,
		start,
		end: start + 1.5 * H,
		version: 1,
		moved: false,
		state: players.length === 4 ? 'full' : 'short',
		players: players.map(person),
		line: [],
		match_spectator_token: null,
		...over
	};
}

function view(games: GameOut[], pieces: PieceOut[] = [], over: Partial<BoardView> = {}): BoardView {
	const ids = new Set([...games.flatMap((g) => g.players.map((p) => p.user_id)), ...pieces.map((p) => p.user_id)]);
	return {
		board: {
			id: 'b',
			token: 't',
			title: 'Thursday padel',
			place: null,
			time_zone: 'UTC',
			starts_at: T0,
			ends_at: T0 + 7 * 86400,
			creator_user_id: 'ana',
			creator_name: 'Ana',
			over: false
		},
		me: { user_id: 'ana', member: true, creator: true },
		members: [...ids].map((id, i) => ({ user_id: id, name: id.toUpperCase(), joined_at: i })),
		pieces,
		games,
		requests: [],
		...over
	};
}

const mine = () => game('g1', G, ['ana', 'rui', 'marta', 'joana']);
const shown = (v: BoardView, me: string | null, now: number) => {
	const a = gameActions(v, 'g1', me, now);
	return (Object.keys(a) as GameAction[]).filter((k) => a[k].shown);
};

describe('gameActions', () => {
	it('start_shows_from_thirty_minutes_before_until_end', () => {
		const v = view([mine()]);
		const start = (now: number) => gameActions(v, 'g1', 'ana', now).start.shown;
		expect(start(G - 1801)).toBe(false);
		expect(start(G - 1800)).toBe(true);
		expect(start(G + 3600)).toBe(true);
		expect(start(G + 1.5 * H - 1)).toBe(true);
		expect(start(G + 1.5 * H)).toBe(false);
	});

	it('start needs four players', () => {
		const v = view([game('g1', G, ['ana', 'rui', 'marta'])]);
		expect(gameActions(v, 'g1', 'ana', G - 600).start.shown).toBe(false);
	});

	it('the other actions end when the game starts', () => {
		const v = view([mine()]);
		expect(shown(v, 'ana', G - 7200)).toEqual(['leave', 'give', 'swap', 'time']);
		expect(shown(v, 'ana', G + 60)).toEqual(['start']);
	});

	it('give_disabled_when_nobody_can_take', () => {
		const alone = view([mine()]);
		const a = gameActions(alone, 'g1', 'ana', G - 7200).give;
		expect(a.shown).toBe(true);
		expect(a.enabled).toBe(false);
		expect(a.reason).toBe('Nobody can take your place right now. Use Leave instead.');
		// Somebody in line is enough.
		const line = view([{ ...mine(), line: ['zoe'] }]);
		expect(gameActions(line, 'g1', 'ana', G - 7200).give.enabled).toBe(true);
	});

	it('give is enabled by an overlapping game whose player can swap in', () => {
		const other = game('g2', G + H, ['zoe', 'tom', 'eva', 'max']);
		// Zoe is free for my game; Ana is free for g2.
		const pieces = [piece('zoe', G, G + 1.5 * H), piece('ana', G + H, G + 2.5 * H)];
		expect(gameActions(view([mine(), other], pieces), 'g1', 'ana', G - 7200).give.enabled).toBe(true);
		// Without Ana's time for the other game, there is nobody to trade with.
		expect(gameActions(view([mine(), other], [pieces[0]!]), 'g1', 'ana', G - 7200).give.enabled).toBe(false);
	});

	it('give is disabled inside the last hour, which the server refuses', () => {
		const v = view([{ ...mine(), line: ['zoe'] }]);
		const a = gameActions(v, 'g1', 'ana', G - 1800).give;
		expect(a.enabled).toBe(false);
		expect(a.reason).toBe('Too late to give your place: the game starts within the hour.');
	});

	it('take_only_for_short_games_i_am_not_in', () => {
		const short = view([game('g1', G, ['rui', 'marta', 'joana'])]);
		expect(shown(short, 'ana', G - 7200)).toEqual(['take']);
		expect(shown(short, 'rui', G - 7200)).toEqual(['leave', 'give', 'swap', 'time']);
		expect(shown(short, null, G - 7200)).toEqual([]);
		// A full game has no open place, and a started one cannot be joined.
		expect(shown(view([mine()]), 'zoe', G - 7200)).toEqual([]);
		expect(shown(short, 'ana', G + 60)).toEqual([]);
	});

	it('take is disabled while I am in another game then', () => {
		const clash = game('g2', G - H, ['ana', 'a', 'b', 'c']);
		const v = view([game('g1', G, ['rui', 'marta', 'joana']), clash]);
		const a = gameActions(v, 'g1', 'ana', G - 7200).take;
		expect(a).toMatchObject({ shown: true, enabled: false, reason: 'You are in another game at that time.' });
	});

	it('nothing_on_an_over_board', () => {
		const v = view([mine()], [], { board: { ...view([]).board, over: true } });
		expect(shown(v, 'ana', G - 600)).toEqual([]);
	});

	it('nothing for a played or past game, or an unknown one', () => {
		expect(shown(view([{ ...mine(), state: 'played' }]), 'ana', G - 600)).toEqual([]);
		expect(shown(view([{ ...mine(), state: 'past' }]), 'ana', G - 600)).toEqual([]);
		expect(shown(view([]), 'ana', G - 600)).toEqual([]);
	});

	it('swap is disabled with nobody to swap with', () => {
		const a = gameActions(view([mine()]), 'g1', 'ana', G - 7200).swap;
		expect(a.enabled).toBe(false);
		expect(a.reason).toBe('Nobody in an overlapping game can swap with you.');
	});
});

describe('swapTargets', () => {
	const other = game('g2', G + H, ['zoe', 'tom', 'eva', 'max']);
	const free = [
		piece('ana', G + H, G + 2.5 * H),
		piece('zoe', G, G + 1.5 * H),
		piece('tom', G, G + 1.5 * H),
		// Eva's time stops short of my game.
		piece('eva', G + H, G + 2.5 * H)
	];

	it('swap_targets_are_players_of_overlapping_games_i_am_free_for', () => {
		const out = swapTargets(view([mine(), other], free), 'g1', 'ana');
		expect(out).toEqual([
			{ gameId: 'g2', userId: 'zoe', name: 'ZOE' },
			{ gameId: 'g2', userId: 'tom', name: 'TOM' }
		]);
	});

	it('needs me to be free for their game', () => {
		const out = swapTargets(view([mine(), other], free.slice(1)), 'g1', 'ana');
		expect(out).toEqual([]);
	});

	it('leaves out games that do not overlap, started games and played ones', () => {
		const later = game('g3', G + 3 * H, ['zoe', 'tom', 'eva', 'max']);
		const played = { ...other, state: 'played' as const };
		expect(swapTargets(view([mine(), later], free), 'g1', 'ana')).toEqual([]);
		expect(swapTargets(view([mine(), played], free), 'g1', 'ana')).toEqual([]);
	});

	it('is empty for a game I am not in', () => {
		expect(swapTargets(view([mine(), other], free), 'g2', 'ana')).toEqual([]);
	});
});

describe('timeOptions', () => {
	it('lists half hours an hour or more ahead, up to the week end, minus where players are busy', () => {
		const now = G - 5 * H - 600; // 14:50 Wed
		const v = view([mine()]);
		const out = timeOptions(v, 'g1', now, 3600);
		// The first is 16:00: ceil((14:50 + 1h) / 30 min).
		expect(out[0]).toBe(G - 4 * H);
		expect(timeOptions(v, 'g1', now, 5400)).not.toContain(G); // the game's own time is not a change
		expect(out.every((t) => t % 1800 === 0)).toBe(true);
		expect(out[out.length - 1]).toBe(T0 + 7 * 86400 - 3600);
		expect(timeOptions(v, 'g1', now, 5400).at(-1)).toBe(T0 + 7 * 86400 - 5400);
	});

	it('leaves out starts where a player is in another game', () => {
		const clash = game('g2', G - 6 * H, ['rui', 'a', 'b', 'c']);
		const out = timeOptions(view([mine(), clash]), 'g1', G - 12 * H, 3600);
		expect(out).not.toContain(G - 6 * H);
		expect(out).not.toContain(G - 6 * H + 3600);
		expect(out).toContain(G - 6 * H + 1.5 * H);
	});

	it('follows the board grid in a quarter-hour zone', () => {
		const v = view([mine()], [], { board: { ...view([]).board, starts_at: T0 + 900, ends_at: T0 + 900 + 7 * 86400 } });
		const out = timeOptions(v, 'g1', G - 5 * H, 3600);
		expect(out.every((t) => (t - T0 - 900) % 1800 === 0)).toBe(true);
	});
});

describe('requestCards', () => {
	const asked = (over: Partial<RequestOut>): RequestOut => ({
		id: 'r1',
		kind: 'swap',
		from: person('rui'),
		game_id: 'g2',
		other_game_id: 'g1',
		new_start: null,
		new_end: null,
		needs_my_answer: true,
		...over
	});

	it('shows a swap as my game against the asker game', () => {
		const other = game('g2', G + H, ['rui', 'a', 'b', 'c']);
		const v = view([mine(), other], [], { requests: [asked({})] });
		expect(requestCards(v)).toEqual([
			{ id: 'r1', kind: 'swap', from: 'RUI', current: [G, G + 1.5 * H], proposed: [G + H, G + 2.5 * H] }
		]);
	});

	it('shows a time change as the game now against the proposed time', () => {
		const v = view([mine()], [], {
			requests: [asked({ kind: 'time', game_id: 'g1', other_game_id: null, new_start: G + H, new_end: G + 2 * H })]
		});
		expect(requestCards(v)).toEqual([
			{ id: 'r1', kind: 'time', from: 'RUI', current: [G, G + 1.5 * H], proposed: [G + H, G + 2 * H] }
		]);
	});

	it('skips requests I asked and ones whose game is gone', () => {
		const v = view([mine()], [], {
			requests: [asked({ needs_my_answer: false }), asked({ id: 'r2', game_id: 'zz' })]
		});
		expect(requestCards(v)).toEqual([]);
	});
});
