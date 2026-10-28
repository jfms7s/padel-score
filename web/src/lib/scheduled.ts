// Scheduled matches ("Play later" on New match and the match's join page): the calls, the
// request a setup becomes, what a viewer may do on a match and the copy helpers they share.
//
// The match's page is `/j/{join_token}` (`getJoin` in signups.ts opens it). Joining and
// leaving use the same link as events; the owner's routes are by id. Every call here is a
// signed-in call, so a 401 sends the browser to sign-in.

import { accountRequest, ApiError } from './api';
import { dayLabel, fromLocalInputs, monthYear, timeText } from './dates';
import { toFormat, type Setup } from './setup';
import { signupMessage } from './signups';
import { strings } from './strings';
import type { Listing, NewScheduled, ScheduledChange, ScheduledView, StartPlayers } from './types';

const enc = encodeURIComponent;
const signedIn = { signedIn: true } as const;
const scheduled = (id: string, rest = '') => `/api/scheduled/${enc(id)}${rest}`;

/** Half an hour before the start, the owner can start the match. */
export const START_WINDOW_SECS = 1800;
/** A match that was not started is past this long after its start (server `MATCH_SECS`). */
export const MATCH_SECS = 5400;

// ---- calls -------------------------------------------------------------------------------

/** 409 `too_many_scheduled`; 422 `starts_at` for a start that is not in the future. */
export function createScheduled(req: NewScheduled): Promise<ScheduledView> {
	return accountRequest('/api/scheduled', { ...signedIn, method: 'POST', body: req, ok: 201 });
}

/** 409 `match_closed` once cancelled or started. */
export function updateScheduled(id: string, change: ScheduledChange): Promise<ScheduledView> {
	return accountRequest(scheduled(id), { ...signedIn, method: 'PATCH', body: change });
}

export function cancelScheduled(id: string): Promise<ScheduledView> {
	return accountRequest(scheduled(id, '/cancel'), { ...signedIn, method: 'POST' });
}

/** Frees a place (the first in line takes it); 404 for an empty one. */
export function removeFromScheduled(id: string, slot: number): Promise<ScheduledView> {
	return accountRequest(scheduled(id, '/remove'), { ...signedIn, method: 'POST', body: { slot } });
}

/** The four players for New match; 409 `not_yet`, `game_over`, `match_short`, `match_closed`. */
export function startScheduled(id: string): Promise<StartPlayers> {
	return accountRequest(scheduled(id, '/start'), { ...signedIn, method: 'POST' });
}

/** Takes the next open place, or a spot in the line; 409 `already_in`, `line_full`, `match_closed`. */
export async function joinMatch(token: string): Promise<void> {
	await accountRequest(`/api/join/${enc(token)}`, { ...signedIn, method: 'POST', body: { as: 'solo' } });
}

export async function leaveMatch(token: string): Promise<void> {
	await accountRequest(`/api/join/${enc(token)}`, { ...signedIn, method: 'DELETE', ok: 204 });
}

/** What to tell the user about a failed scheduled-match call. */
export function scheduledMessage(e: unknown): string {
	if (e instanceof ApiError) {
		const errors: Record<string, string> = strings.scheduled.errors;
		const fields: Record<string, string> = strings.scheduled.fields;
		if (e.code === 'validation' && e.field !== undefined && Object.hasOwn(fields, e.field)) return fields[e.field]!;
		if (Object.hasOwn(errors, e.code)) return errors[e.code]!;
		const reasons: Record<string, string> = strings.scheduled.invalidMatch;
		if (e.code === 'invalid_match' && Object.hasOwn(reasons, e.message)) return reasons[e.message]!;
	}
	return signupMessage(e);
}

// ---- Play later --------------------------------------------------------------------------

/** The Play later fields of New match: the date and time are the browser's date and time inputs. */
export interface LaterDraft {
	on: boolean;
	date: string;
	time: string;
	place: string;
	listing: Listing;
}

export function emptyLater(): LaterDraft {
	return { on: false, date: '', time: '', place: '', listing: 'private' };
}

/** The start the inputs name, read in the zone; null when one is empty or it is not after `now`. */
export function laterStart(draft: LaterDraft, now: number, timeZone?: string): number | null {
	if (draft.date === '' || draft.time === '') return null;
	const at = fromLocalInputs(draft.date, draft.time, timeZone);
	return at > now ? at : null;
}

/**
 * The request for a setup: its rules, and its picked players as the four places in slot order
 * (team A then team B). A slot with no picked player stays open.
 */
export function toNewScheduled(
	setup: Setup,
	later: { startsAt: number; place: string; listing: Listing }
): NewScheduled {
	const { a, b } = setup.players;
	const req: NewScheduled = {
		starts_at: later.startsAt,
		listing: later.listing,
		format: toFormat(setup),
		places: [a[0], a[1], b[0], b[1]]
	};
	const place = later.place.trim();
	if (place !== '') req.place = place;
	return req;
}

// ---- the match page ----------------------------------------------------------------------

export interface MatchActions {
	join: boolean;
	leave: boolean;
	start: boolean;
	edit: boolean;
}

/** What the viewer may do now. The server decides again on every call. */
export function matchActions(view: ScheduledView, now: number): MatchActions {
	const me = view.me;
	const live = view.state === 'open' || view.state === 'full';
	const anOpenPlace = view.places.some((p) => p.name === null);
	const isIn = me !== null && (me.slot !== null || me.in_line);
	const owner = me?.owner === true;
	const filled = view.places.length === 4 && view.places.every((p) => p.name !== null);
	const inWindow = now >= view.starts_at - START_WINDOW_SECS && now < view.starts_at + MATCH_SECS;
	return {
		// Someone on the line may still take an open place by hand.
		join: me !== null && me.slot === null && live && (!me.in_line || anOpenPlace),
		leave: isIn && live,
		start: owner && live && filled && inWindow,
		edit: owner && live
	};
}

/** "Wed 4 Nov 2026 · 20:00 · Club Norte". */
export function matchWhen(view: Pick<ScheduledView, 'starts_at' | 'place'>, timeZone?: string): string {
	const parts = [
		`${dayLabel(view.starts_at, timeZone)} ${monthYear(view.starts_at, timeZone)}`,
		timeText(view.starts_at, timeZone)
	];
	if (view.place !== null) parts.push(view.place);
	return parts.join(' · ');
}

/**
 * The edit of a match from its form: only the start or the place that differ from `view`, or
 * null when nothing does. An emptied place clears it.
 */
export function scheduledChange(
	view: ScheduledView,
	form: { date: string; time: string; place: string },
	timeZone?: string
): ScheduledChange | null {
	const change: ScheduledChange = {};
	if (form.date !== '' && form.time !== '') {
		const at = fromLocalInputs(form.date, form.time, timeZone);
		if (at !== view.starts_at) change.starts_at = at;
	}
	const place = form.place.trim();
	if (place !== (view.place ?? '')) change.place = place === '' ? null : place;
	return Object.keys(change).length > 0 ? change : null;
}

/** The match's join link (the same kind as an event's). */
export { joinLink as matchLink } from './signups';
