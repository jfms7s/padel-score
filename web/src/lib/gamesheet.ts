// What the game sheet offers, from the board view alone. It mirrors the server's rules
// (crates/server/src/boards/formation.rs, actions.rs) so a button is greyed out with a reason
// instead of failing; the server stays the judge, and answers 409 when this guess is wrong.

import { mergedRanges, SLOT_SECS } from './grid';
import { strings } from './strings';
import type { BoardView, GameOut } from './types';

export type GameAction = 'leave' | 'give' | 'swap' | 'time' | 'take' | 'start';

export interface ActionState {
	shown: boolean;
	enabled: boolean;
	/** Why a shown action is disabled. */
	reason?: string;
}

/** A game must start this long from now to be given away, swapped into or moved to. */
const LEAD_SECS = 3600;
/** Start match opens this long before the start. */
const START_WINDOW_SECS = 1800;
const PLACES = 4;

const overlaps = (aStart: number, aEnd: number, bStart: number, bEnd: number) => aStart < bEnd && bStart < aEnd;
const isIn = (g: GameOut, user: string) => g.players.some((p) => p.user_id === user);

/** Do the user's pieces (touching ones joined) cover all of [start, end)? */
function covered(view: BoardView, user: string, start: number, end: number): boolean {
	return mergedRanges(view.pieces, user).some((r) => r.start <= start && end <= r.end);
}

/** Is the user in a game that overlaps [start, end), other than `except`? */
function busy(view: BoardView, user: string, start: number, end: number, except?: string): boolean {
	return view.games.some((g) => g.id !== except && overlaps(g.start, g.end, start, end) && isIn(g, user));
}

const freeFor = (view: BoardView, user: string, start: number, end: number, except?: string) =>
	covered(view, user, start, end) && !busy(view, user, start, end, except);

/** A game another game's players could still trade with: not played, not started. */
const tradable = (g: GameOut, now: number) => g.state !== 'played' && g.state !== 'past' && g.start > now;

/** Could someone take `me`'s place: the line, or a player of an overlapping game who can trade? */
function canGive(view: BoardView, game: GameOut, me: string, now: number): boolean {
	if (game.line.some((u) => u !== me)) return true;
	return view.games.some(
		(o) =>
			o.id !== game.id &&
			tradable(o, now) &&
			o.start >= now + LEAD_SECS &&
			overlaps(o.start, o.end, game.start, game.end) &&
			freeFor(view, me, o.start, o.end, game.id) &&
			o.players.some((p) => freeFor(view, p.user_id, game.start, game.end, o.id))
	);
}

/** Players of overlapping games that `me` could ask to swap with, in game then seat order; with `now`, games that started are left out. */
export function swapTargets(
	view: BoardView,
	gameId: string,
	me: string,
	now = Number.NEGATIVE_INFINITY
): { gameId: string; userId: string; name: string }[] {
	const game = view.games.find((g) => g.id === gameId);
	if (!game || !isIn(game, me) || !tradable(game, now)) return [];
	return view.games
		.filter((o) => o.id !== game.id && tradable(o, now) && overlaps(o.start, o.end, game.start, game.end))
		.filter((o) => freeFor(view, me, o.start, o.end, game.id))
		.flatMap((o) =>
			o.players
				.filter((p) => p.user_id !== me && freeFor(view, p.user_id, game.start, game.end, o.id))
				.map((p) => ({ gameId: o.id, userId: p.user_id, name: p.name }))
		);
}

/**
 * The starts a game can move to for a length in seconds: half hours on the board's grid from an
 * hour ahead to the week's end, where nobody in the game is in another game and the game does
 * not stay where it is.
 */
export function timeOptions(view: BoardView, gameId: string, now: number, length: number): number[] {
	const game = view.games.find((g) => g.id === gameId);
	if (!game) return [];
	const { starts_at, ends_at } = view.board;
	const first = starts_at + Math.max(0, Math.ceil((now + LEAD_SECS - starts_at) / SLOT_SECS)) * SLOT_SECS;
	const out: number[] = [];
	for (let t = first; t + length <= ends_at; t += SLOT_SECS) {
		if (t === game.start && t + length === game.end) continue;
		if (game.players.some((p) => busy(view, p.user_id, t, t + length, game.id))) continue;
		out.push(t);
	}
	return out;
}

const none: ActionState = { shown: false, enabled: false };

/** Which actions the game sheet shows `me` for a game, and which of them work right now. */
export function gameActions(
	view: BoardView,
	gameId: string,
	me: string | null,
	now: number
): Record<GameAction, ActionState> {
	const hidden = { leave: none, give: none, swap: none, time: none, take: none, start: none };
	const game = view.games.find((g) => g.id === gameId);
	if (!game || me === null || view.board.over || game.state === 'played' || game.state === 'past') return hidden;
	if (now >= game.end) return hidden;
	const reasons = strings.gameSheet.reasons;
	const inGame = isIn(game, me);
	const upcoming = game.start > now;
	const state = (shown: boolean, enabled = true, reason?: string): ActionState =>
		shown ? (enabled || reason === undefined ? { shown, enabled } : { shown, enabled, reason }) : none;
	const lateGive = game.start < now + LEAD_SECS;
	const giveOk = canGive(view, game, me, now);
	const swapOk = swapTargets(view, gameId, me, now).length > 0;
	const timeOk = [3600, 5400].some((len) => timeOptions(view, gameId, now, len).length > 0);
	return {
		leave: state(inGame && upcoming),
		give: state(inGame && upcoming, !lateGive && giveOk, lateGive ? reasons.giveLate : reasons.give),
		swap: state(inGame && upcoming, swapOk, reasons.swap),
		time: state(inGame && upcoming, timeOk, reasons.time),
		take: state(
			!inGame && upcoming && game.players.length < PLACES,
			!busy(view, me, game.start, game.end),
			reasons.take
		),
		start: state(inGame && game.players.length === PLACES && now >= game.start - START_WINDOW_SECS)
	};
}

/** What `RequestCard` shows: a request waiting for my answer, with the times it compares. */
export interface RequestCardData {
	id: string;
	kind: 'swap' | 'time';
	from: string;
	current: [number, number];
	proposed: [number, number];
}

/** The pending requests the viewer must answer, with game times from the view. */
export function requestCards(view: BoardView): RequestCardData[] {
	const game = (id: string | null) => view.games.find((g) => g.id === id);
	const out: RequestCardData[] = [];
	for (const r of view.requests) {
		if (!r.needs_my_answer) continue;
		if (r.kind === 'swap') {
			// The asker's game is `game_id`; mine is the other one.
			const mineGame = game(r.other_game_id);
			const theirs = game(r.game_id);
			if (mineGame && theirs) {
				out.push({
					id: r.id,
					kind: 'swap',
					from: r.from.name,
					current: [mineGame.start, mineGame.end],
					proposed: [theirs.start, theirs.end]
				});
			}
		} else {
			const g = game(r.game_id);
			if (g && r.new_start !== null && r.new_end !== null) {
				out.push({
					id: r.id,
					kind: 'time',
					from: r.from.name,
					current: [g.start, g.end],
					proposed: [r.new_start, r.new_end]
				});
			}
		}
	}
	return out;
}
