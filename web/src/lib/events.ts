// Events (phase 6): the coordinator's API, the court phone's and the event viewing page's calls,
// and the pure helpers their screens share (typed scores, team and court lines, final places).
//
// Coordinator calls go through `accountRequest`, so a 401 sends the browser to sign-in. Court and
// viewing calls need no account: a court phone is anyone holding the court's link.

import { accountRequest, ApiError, courtUrl, plainRequest } from './api';
import { authMessage } from './auth';
import { strings } from './strings';
import { pointLabel } from './view';
import type {
	CourtOut,
	CourtState,
	EventChange,
	EventDetail,
	EventMatch,
	EventResultRequest,
	EventSummary,
	EventTeam,
	EventView,
	Format,
	InvalidScore,
	MatchChange,
	MatchView,
	NewEvent,
	NewEventMatch,
	NewTeam,
	Place,
	Score,
	SetScore,
	Team,
	TeamChange,
	TypedScore
} from './types';

const enc = encodeURIComponent;
const event = (id: string, rest = '') => `/api/events/${enc(id)}${rest}`;
const signedIn = { signedIn: true } as const;

// ---- coordinator -------------------------------------------------------------------------

/** The coordinator's events, newest first. */
export async function listEvents(): Promise<EventSummary[]> {
	const page = await accountRequest<{ items: EventSummary[] }>('/api/events', signedIn);
	return page.items;
}

export function createEvent(req: NewEvent): Promise<EventDetail> {
	return accountRequest('/api/events', { ...signedIn, method: 'POST', body: req, ok: 201 });
}

export function getEvent(id: string): Promise<EventDetail> {
	return accountRequest(event(id), signedIn);
}

/** Only in setup (409 `event_started` after). */
export function updateEvent(id: string, change: EventChange): Promise<EventDetail> {
	return accountRequest(event(id), { ...signedIn, method: 'PATCH', body: change });
}

/** Only in setup (409 `event_started` after). */
export async function deleteEvent(id: string): Promise<void> {
	await accountRequest(event(id), { ...signedIn, method: 'DELETE', ok: 204 });
}

/**
 * Needs two teams and a court (409 `too_few_teams`, `no_courts`); a non-stop also an even
 * number of teams and a schedule that is not too big (409 `odd_teams`, `schedule_too_big`).
 */
export function startEvent(id: string): Promise<EventDetail> {
	return accountRequest(event(id, '/start'), { ...signedIn, method: 'POST' });
}

/** Finish with the final places (see `placesFromOrder`); they must tile the field. */
export function finishEvent(id: string, places?: Place[]): Promise<EventDetail> {
	const body = places === undefined ? {} : { places };
	return accountRequest(event(id, '/finish'), { ...signedIn, method: 'POST', body });
}

export function addTeam(id: string, req: NewTeam): Promise<EventTeam> {
	return accountRequest(event(id, '/teams'), { ...signedIn, method: 'POST', body: req, ok: 201 });
}

export function updateTeam(id: string, teamId: string, change: TeamChange): Promise<EventTeam> {
	return accountRequest(event(id, `/teams/${enc(teamId)}`), { ...signedIn, method: 'PATCH', body: change });
}

export async function deleteTeam(id: string, teamId: string): Promise<void> {
	await accountRequest(event(id, `/teams/${enc(teamId)}`), { ...signedIn, method: 'DELETE', ok: 204 });
}

/** Without a name the server calls it "Court {n}". */
export function addCourt(id: string, name?: string): Promise<CourtOut> {
	const body = name === undefined ? {} : { name };
	return accountRequest(event(id, '/courts'), { ...signedIn, method: 'POST', body, ok: 201 });
}

export function renameCourt(id: string, courtId: string, name: string): Promise<CourtOut> {
	return accountRequest(event(id, `/courts/${enc(courtId)}`), { ...signedIn, method: 'PATCH', body: { name } });
}

/** Not while a match is on it (409 `court_in_use`). */
export async function deleteCourt(id: string, courtId: string): Promise<void> {
	await accountRequest(event(id, `/courts/${enc(courtId)}`), { ...signedIn, method: 'DELETE', ok: 204 });
}

/** A new scoring link for the court: the old one stops working. */
export function rotateCourt(id: string, courtId: string): Promise<CourtOut> {
	return accountRequest(event(id, `/courts/${enc(courtId)}/rotate`), { ...signedIn, method: 'POST' });
}

/** Move the court on to its next match (409 `match_not_finished`). */
export function advanceCourt(id: string, courtId: string): Promise<EventDetail> {
	return accountRequest(event(id, `/courts/${enc(courtId)}/advance`), { ...signedIn, method: 'POST' });
}

/** Only while running (409 `event_not_running`). */
export function createEventMatch(id: string, req: NewEventMatch): Promise<EventMatch> {
	return accountRequest(event(id, '/matches'), { ...signedIn, method: 'POST', body: req, ok: 201 });
}

/** Put a match on a court, move it, or take it off any court (`courtId` null). */
export function assignMatch(id: string, matchId: string, courtId: string | null, label?: string): Promise<EventMatch> {
	const body: MatchChange = label === undefined ? { court_id: courtId } : { court_id: courtId, label };
	return accountRequest(event(id, `/matches/${enc(matchId)}`), { ...signedIn, method: 'PATCH', body });
}

/**
 * The coordinator types a match's result. It replaces whatever was scored and locks the match
 * against its scoring links. A refused score throws an ApiError `invalid_score` with `reason`.
 */
export function postEventResult(id: string, matchId: string, req: EventResultRequest): Promise<MatchView> {
	return accountRequest(event(id, `/matches/${enc(matchId)}/result`), { ...signedIn, method: 'POST', body: req });
}

/** The coordinator's SSE stream of `event` frames (each an EventDetail). */
export function eventStreamUrl(id: string): string {
	return event(id, '/stream');
}

// ---- court phone and viewing page --------------------------------------------------------

/** What a court link shows; a scoring link also gets the current match's scoring token. */
export function getCourt(token: string): Promise<CourtState> {
	return plainRequest(courtUrl(token));
}

/** The court phone moves on to the next match (scoring link only; 409 `match_not_finished`). */
export function advanceByCourt(token: string): Promise<CourtState> {
	return plainRequest(courtUrl(token, '/advance'), 200, { method: 'POST' });
}

/** The court's SSE stream of `court` frames (each a CourtState). */
export function courtStreamUrl(token: string): string {
	return courtUrl(token, '/stream');
}

/** The whole event for a spectator: no scoring links anywhere. */
export function getEventView(token: string): Promise<EventView> {
	return plainRequest(`/api/event-view/${enc(token)}`);
}

/** The viewing page's SSE stream of `event` frames (each an EventView). */
export function eventViewStreamUrl(token: string): string {
	return `/api/event-view/${enc(token)}/stream`;
}

// ---- typed scores ------------------------------------------------------------------------

/**
 * Why typed text isn't a score yet. `needs_overtime_winner` carries the score so far: ask who won
 * the overtime game, then `withOvertimeWinner`.
 */
export type ParsedScore =
	| { ok: TypedScore }
	| { error: 'empty' | 'syntax' | 'needs_tiebreak' }
	| { error: 'needs_overtime_winner'; score: TypedScore };

interface RawSet {
	a: number;
	b: number;
	/** The loser's tie-break points, from `7-6(5)`. */
	loser: number | null;
}

/** One set: `6-4`, `6–4` or `6:4`, maybe followed by the tie-break loser's points `(5)`. */
const SET = /(\d{1,3})\s*[-–:]\s*(\d{1,3})(?:\s*\((\d{1,3})\))?/y;
const SEPARATOR = /[\s,]/;
/** No set has more games than this; more is a typo, not a score for the engine to judge. */
const MAX_SET_GAMES = 99;

/** The sets in `text`, separated by spaces or commas; null when it doesn't read as sets. */
function scan(text: string): RawSet[] | null {
	const sets: RawSet[] = [];
	let i = 0;
	for (;;) {
		while (i < text.length && SEPARATOR.test(text[i]!)) i++;
		if (i >= text.length) return sets;
		SET.lastIndex = i;
		const m = SET.exec(text);
		if (!m) return null;
		i = SET.lastIndex;
		if (i < text.length && !SEPARATOR.test(text[i]!)) return null;
		sets.push({ a: Number(m[1]), b: Number(m[2]), loser: m[3] === undefined ? null : Number(m[3]) });
	}
}

/**
 * Read a typed final score, such as `6-4 3-6 10-7` or `5-3`, into the shape the engine and the
 * server take. It only shapes: whether the score could really have happened is `validateFinal`'s
 * call.
 *
 * - A 7-6 set needs its tie-break as the loser's points, `7-6(5)`; the winner's are
 *   max(7, loser + 2).
 * - Best of 3 with a super tie-break: at one set all, the third "set" is the super tie-break's
 *   points, `10-7`.
 * - A timed match is one `a-b` of games. Level with draws off needs the overtime game's winner.
 */
export function parseTypedScore(text: string, format: Format): ParsedScore {
	const raw = scan(text);
	if (raw === null) return { error: 'syntax' };
	if (raw.length === 0) return { error: 'empty' };

	if ('timed' in format) {
		const only = raw[0]!;
		if (raw.length > 1 || only.loser !== null) return { error: 'syntax' };
		const score: TypedScore = { timed: { a: only.a, b: only.b, overtime_winner: null } };
		if (only.a === only.b && !format.timed.draws_allowed) return { error: 'needs_overtime_winner', score };
		return { ok: score };
	}

	const rules = format.sets;
	const superDecider = rules.sets === 'best_of_3' && rules.deciding === 'super_tie_break';
	const won = [0, 0];
	const sets: SetScore[] = [];
	let needsTiebreak = false;
	for (const { a, b, loser } of raw) {
		let set: SetScore;
		if (superDecider && won[0] === 1 && won[1] === 1) {
			if (loser !== null) return { error: 'syntax' };
			set = { a: a > b ? 1 : 0, b: b > a ? 1 : 0, tiebreak: [a, b] };
		} else {
			if (a > MAX_SET_GAMES || b > MAX_SET_GAMES) return { error: 'syntax' };
			const tiebreakSet = Math.max(a, b) === 7 && Math.min(a, b) === 6;
			if (loser !== null && !tiebreakSet) return { error: 'syntax' };
			if (tiebreakSet && loser === null) needsTiebreak = true;
			const winner = loser === null ? 0 : Math.max(7, loser + 2);
			const tiebreak: [number, number] | null = loser === null ? null : a > b ? [winner, loser] : [loser, winner];
			set = { a, b, tiebreak };
		}
		if (set.a > set.b) won[0]!++;
		else if (set.b > set.a) won[1]!++;
		sets.push(set);
	}
	return needsTiebreak ? { error: 'needs_tiebreak' } : { ok: { sets } };
}

/** A level timed score with its overtime game's winner; a sets score is left as it is. */
export function withOvertimeWinner(score: TypedScore, team: Team): TypedScore {
	return 'timed' in score ? { timed: { ...score.timed, overtime_winner: team } } : score;
}

// ---- lines -------------------------------------------------------------------------------

/** A team as shown: its custom name, or "Ana / Marta" (the server builds it). */
/** How many of the event's matches aren't finished (their court phones stop at Finish). */
export function unfinishedCount(matches: readonly Pick<EventMatch, 'state'>[]): number {
	return matches.filter((m) => m.state !== 'finished').length;
}

export function teamText(t: EventTeam): string {
	return t.name;
}

/** "Ana / Marta v Rui / João". */
export function matchText(m: Pick<EventMatch, 'team_a' | 'team_b'>, teams: readonly EventTeam[]): string {
	const name = (id: string) => {
		const t = teams.find((x) => x.id === id);
		return t ? teamText(t) : strings.events.unknownTeam;
	};
	return strings.events.versus(name(m.team_a), name(m.team_b));
}

/**
 * The court card's status line: no phone yet (hand over the scoring code), the phone waiting
 * for a match, or the phone and the match on court.
 */
export function courtStatusText(c: CourtOut, matches: readonly EventMatch[], teams: readonly EventTeam[]): string {
	if (c.phones === 0) return strings.events.noPhone;
	const current = c.current === null ? undefined : matches.find((m) => m.id === c.current);
	if (!current) return strings.events.waitingForMatch;
	return strings.events.phoneConnected(matchText(current, teams));
}

// ---- final places ------------------------------------------------------------------------

/**
 * Final places from the teams in finishing order, in groups: a group of k teams shares k places
 * (`[[a], [b], [c, d]]` is 1st, 2nd, then 3rd-4th for both c and d). Empty groups are skipped.
 */
export function placesFromOrder(groups: readonly (readonly string[])[]): Place[] {
	const places: Place[] = [];
	let next = 1;
	for (const group of groups) {
		if (group.length === 0) continue;
		const place_to = next + group.length - 1;
		for (const team_id of group) places.push({ team_id, place: next, place_to });
		next = place_to + 1;
	}
	return places;
}

/**
 * Teams in finishing order, grouped by ties: `tiedWithNext[i]` joins `order[i]` and
 * `order[i + 1]` in one shared place. The result goes to `placesFromOrder`.
 */
export function groupsFromTies(order: readonly string[], tiedWithNext: readonly boolean[]): string[][] {
	const groups: string[][] = [];
	order.forEach((id, i) => {
		const last = groups.at(-1);
		if (last && i > 0 && tiedWithNext[i - 1]) last.push(id);
		else groups.push([id]);
	});
	return groups;
}

/** `list` with the item at `from` moved to `to`; out-of-range indexes leave it as it is. */
export function moveItem<T>(list: readonly T[], from: number, to: number): T[] {
	const out = [...list];
	if (from < 0 || from >= out.length || to < 0 || to >= out.length) return out;
	const [item] = out.splice(from, 1);
	out.splice(to, 0, item as T);
	return out;
}

// ---- links -------------------------------------------------------------------------------

/** A court's two QR links: the court phone's scoring page and its viewing page. */
export function courtLinks(
	origin: string,
	court: Pick<CourtOut, 'scorer_token' | 'spectator_token'>
): { scoring: string; viewing: string } {
	return { scoring: `${origin}/c/${enc(court.scorer_token)}`, viewing: `${origin}/cv/${enc(court.spectator_token)}` };
}

/** The whole event's viewing link, for spectators. */
export function eventLink(origin: string, spectatorToken: string): string {
	return `${origin}/e/${enc(spectatorToken)}`;
}

/** One match's viewing page. */
export function matchViewPath(spectatorToken: string): string {
	return `/v/${enc(spectatorToken)}`;
}

// ---- words for the screens ---------------------------------------------------------------

/** The rules on one line: "Best of 3, super tie-break, golden point", "Timed, golden point, no draws". */
export function rulesText(format: Format): string {
	const t = strings.events;
	if ('timed' in format) {
		const { deuce, draws_allowed } = format.timed;
		return [t.timed, strings.deuceInLine[deuce], draws_allowed ? t.drawsAllowed : t.noDraws].join(', ');
	}
	const { deuce, sets, deciding } = format.sets;
	const parts: string[] = [strings.setup.setsOptions[sets]];
	if (sets === 'best_of_3') parts.push(t.rulesDeciding[deciding]);
	parts.push(strings.deuceInLine[deuce]);
	return parts.join(', ');
}

/** A live match's score: the sets' games (or a timed match's games), then the point. */
export function liveScoreText(score: Score): string {
	const games =
		score.sets.length > 0 ? score.sets.map((s) => `${s.a}-${s.b}`).join(' ') : `${score.games[0]}-${score.games[1]}`;
	const c = score.current;
	if (c === 'none') return games;
	const point = 'game' in c ? `${pointLabel(c.game.a)}-${pointLabel(c.game.b)}` : `${c.tie_break.a}-${c.tie_break.b}`;
	return strings.events.liveScore(games, point);
}

type InvalidScoreKey = keyof typeof strings.events.invalidScore;

/** Which `strings.events.invalidScore` entry explains `reason`. */
export function invalidScoreKey(reason: InvalidScore): InvalidScoreKey {
	if (typeof reason === 'string') return reason;
	if ('invalid_rules' in reason) return reason.invalid_rules;
	if ('impossible_set' in reason) return 'impossible_set';
	return 'missing_tie_break';
}

/** Why the engine refused a typed score, in words (set numbers counted from one). */
export function invalidScoreText(reason: InvalidScore): string {
	const t = strings.events.invalidScore;
	if (typeof reason === 'string') return t[reason];
	if ('impossible_set' in reason) return t.impossible_set(reason.impossible_set.index + 1);
	if ('missing_tie_break' in reason) return t.missing_tie_break(reason.missing_tie_break.index + 1);
	return t[reason.invalid_rules];
}

/** Why typed text isn't a score yet, in words. */
export function parsedErrorText(error: Exclude<ParsedScore, { ok: TypedScore }>['error']): string {
	return strings.events.parsed[error];
}

/** What to tell the coordinator about a failed event call. */
export function eventMessage(e: unknown): string {
	if (e instanceof ApiError) {
		const errors: Record<string, string> = strings.events.errors;
		const fields: Record<string, string> = strings.events.fields;
		if (e.code === 'invalid_score' && e.reason !== undefined) return invalidScoreText(e.reason);
		if (e.code === 'validation' && e.field !== undefined && Object.hasOwn(fields, e.field)) return fields[e.field]!;
		if (Object.hasOwn(errors, e.code)) return errors[e.code]!;
		const players: Record<string, string> = strings.players.errors;
		if (Object.hasOwn(players, e.code)) return players[e.code]!;
	}
	return authMessage(e);
}

/** Why a typed result was refused, as the result sheet and the match list's field say it. */
export function resultRefusalText(e: unknown): string {
	if (e instanceof ApiError) {
		if (e.code === 'invalid_score' && e.reason !== undefined) return invalidScoreText(e.reason);
		if (e.code === 'conflict') return strings.events.sheet.conflict;
		if (e.code === 'locked') return strings.events.sheet.locked;
	}
	return eventMessage(e);
}

// ---- stream frames -----------------------------------------------------------------------

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/**
 * A loose check of an `event` frame on the coordinator's stream: the parts the screen walks.
 * (No strict guard yet: there are no server samples of these shapes.)
 */
export function isEventDetail(v: unknown): v is EventDetail {
	return (
		isObject(v) &&
		isObject(v.event) &&
		typeof v.event.id === 'string' &&
		typeof v.spectator_token === 'string' &&
		['teams', 'courts', 'matches', 'places'].every((k) => Array.isArray(v[k]))
	);
}
