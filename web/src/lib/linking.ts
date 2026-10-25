// Linking player entries to accounts, claims and the combined history
// (crates/server/src/linking_api.rs), typed, plus the pure helpers the pages share.
//
// Every call goes through api.ts's accountRequest. `signedIn` marks the calls that need a
// session. Codes travel in request bodies only, never in a path or query. `getProfile` is the one
// public call: it needs no credential, so it leaves `signedIn` off and a 401 never redirects.

import { accountRequest, ApiError } from './api';
import { playerMessage } from './players';
import { strings } from './strings';
import type {
	Claim,
	ClaimPreview,
	ClaimTarget,
	CreatedLinkCode,
	LinkedEntry,
	LinkedHistory,
	MyClaim,
	MyLink,
	Player,
	Privacy,
	SlotState,
	Visibility
} from './types';

const enc = encodeURIComponent;
const POST = 'POST' as const;
const DELETE = 'DELETE' as const;

const before = (cursor?: string | null) => (cursor ? `?before=${enc(cursor)}` : '');

// ---- link codes --------------------------------------------------------------------------

/** The coordinator's code for one entry; the server voids the previous one. */
export function createLinkCode(playerId: string): Promise<CreatedLinkCode> {
	return accountRequest(`/api/players/${enc(playerId)}/link-code`, { method: POST, ok: 201, signedIn: true });
}

export function lookupLinkCode(code: string): Promise<ClaimPreview> {
	return accountRequest('/api/link-codes/lookup', { method: POST, body: { code }, signedIn: true });
}

export function acceptLinkCode(code: string): Promise<{ player_id: string }> {
	return accountRequest('/api/link-codes/accept', { method: POST, body: { code }, signedIn: true });
}

/** The coordinator links an entry to their own account. */
export function linkMe(playerId: string): Promise<void> {
	return accountRequest(`/api/players/${enc(playerId)}/link-me`, { method: POST, ok: 204, signedIn: true });
}

/** The coordinator unlinks an entry. */
export function unlink(playerId: string): Promise<void> {
	return accountRequest(`/api/players/${enc(playerId)}/link`, { method: DELETE, ok: 204, signedIn: true });
}

// ---- claims ------------------------------------------------------------------------------

export async function getSlots(spectatorToken: string): Promise<SlotState[]> {
	// Not `signedIn`: the spectator page is public, so a 401 must never send the viewer to sign-in.
	const list = await accountRequest<{ items: SlotState[] }>(`/api/view/${enc(spectatorToken)}/slots`);
	return list.items;
}

export function previewClaim(target: ClaimTarget): Promise<ClaimPreview> {
	return accountRequest('/api/claims/preview', { method: POST, body: target, signedIn: true });
}

/** 201 for a new claim, 200 when an earlier one was reopened or already pending. */
export function createClaim(target: ClaimTarget): Promise<MyClaim> {
	return accountRequest('/api/claims', { method: POST, body: target, ok: [200, 201], signedIn: true });
}

/** Claims on the caller's players, waiting for an answer. */
export async function listClaims(): Promise<Claim[]> {
	return (await accountRequest<{ items: Claim[] }>('/api/claims', { signedIn: true })).items;
}

export function approveClaim(id: string): Promise<void> {
	return accountRequest(`/api/claims/${enc(id)}/approve`, { method: POST, ok: 204, signedIn: true });
}

export function declineClaim(id: string): Promise<void> {
	return accountRequest(`/api/claims/${enc(id)}/decline`, { method: POST, ok: 204, signedIn: true });
}

export async function myClaims(): Promise<MyClaim[]> {
	return (await accountRequest<{ items: MyClaim[] }>('/api/me/claims', { signedIn: true })).items;
}

export function withdrawClaim(id: string): Promise<void> {
	return accountRequest(`/api/me/claims/${enc(id)}`, { method: DELETE, ok: 204, signedIn: true });
}

// ---- my links, history and privacy -------------------------------------------------------

export async function myLinks(): Promise<MyLink[]> {
	return (await accountRequest<{ items: MyLink[] }>('/api/me/links', { signedIn: true })).items;
}

/** The linked player drops an entry from their own account. */
export function unlinkMe(playerId: string): Promise<void> {
	return accountRequest(`/api/me/links/${enc(playerId)}`, { method: DELETE, ok: 204, signedIn: true });
}

/** The caller's combined history, newest first; pass `next` back for the following page. */
export function myResults(cursor?: string | null): Promise<LinkedHistory> {
	return accountRequest(`/api/me/results${before(cursor)}`, { signedIn: true });
}

/** A linked player's combined history as the caller may see it (403 `history_private`). */
export function linkedHistory(playerId: string, cursor?: string | null): Promise<LinkedHistory> {
	return accountRequest(`/api/players/${enc(playerId)}/linked-history${before(cursor)}`, { signedIn: true });
}

export function getPrivacy(): Promise<Privacy> {
	return accountRequest('/api/me/privacy', { signedIn: true });
}

export function setPrivacy(visibility: Visibility): Promise<Privacy> {
	return accountRequest('/api/me/privacy', { method: 'PUT', body: { visibility }, signedIn: true });
}

/** A new profile link; the old one stops working at once. */
export function rotateProfile(): Promise<Privacy> {
	return accountRequest('/api/me/privacy/rotate', { method: POST, signedIn: true });
}

/** Public, no credential: a player's history by profile token (404 unless public). */
export function getProfile(token: string, cursor?: string | null): Promise<LinkedHistory> {
	return accountRequest(`/api/profiles/${enc(token)}${before(cursor)}`);
}

/** What to tell the user about a failed linking call. */
export function linkMessage(e: unknown): string {
	const errors: Record<string, string> = strings.linking.errors;
	if (e instanceof ApiError && Object.hasOwn(errors, e.code)) return errors[e.code]!;
	return playerMessage(e);
}

// ---- pure helpers ------------------------------------------------------------------------

const monthYear = (secs: number) =>
	new Date(secs * 1000).toLocaleDateString('en', { month: 'short', year: 'numeric' });

/** "5 matches · 2 events · Mar 2026 – Oct 2026"; one date when both fall in the same month. */
export function previewText(p: ClaimPreview): string {
	const t = strings.linking;
	if (p.matches === 0 && p.events === 0) return t.nothingYet;
	const parts = [t.match(p.matches)];
	if (p.events > 0) parts.push(t.event(p.events));
	if (p.first_at !== null && p.last_at !== null) {
		const from = monthYear(p.first_at);
		const to = monthYear(p.last_at);
		parts.push(from === to ? from : t.range(from, to));
	}
	return parts.join(' · ');
}

/** "You", "Linked · Rui Costa" or nothing for an unlinked entry. */
export function linkLabel(p: Player): string | null {
	if (!p.link) return null;
	return p.link.is_me ? strings.linking.you : strings.linking.linkedTo(p.link.display_name);
}

/** Claims waiting on the caller, summed over their player list: the badge on the home link. */
export function claimBadgeCount(players: Pick<Player, 'pending_claims'>[]): number {
	return players.reduce((n, p) => n + p.pending_claims, 0);
}

/** "Won", "Lost" or "Drawn": a linked match from the player's side. */
export function outcomeText(e: Pick<LinkedEntry, 'outcome'>): string {
	return strings.history.outcome[e.outcome];
}

// ---- the link code kept across sign-in ---------------------------------------------------

export const PENDING_LINK_KEY = 'padel.pendingLinkCode';

/** How long a stashed code is kept: a link code lasts 10 minutes, so 15 is already generous. */
export const STASH_TTL_MS = 15 * 60 * 1000;

/** Keep a scanned code across sign-in. Best effort: no storage means the code is scanned again. */
export function stashLinkCode(code: string): void {
	try {
		sessionStorage.setItem(PENDING_LINK_KEY, JSON.stringify({ code, at: Date.now() }));
	} catch {
		// Storage blocked: the player scans again after signing in.
	}
}

/** The stashed code, once: reading clears it. Null when none, expired or unreadable. */
export function takeStashedLinkCode(): string | null {
	try {
		const raw = sessionStorage.getItem(PENDING_LINK_KEY);
		sessionStorage.removeItem(PENDING_LINK_KEY);
		if (raw === null) return null;
		const stash = JSON.parse(raw) as { code?: unknown; at?: unknown };
		if (typeof stash.code !== 'string' || typeof stash.at !== 'number') return null;
		return Date.now() - stash.at <= STASH_TTL_MS ? stash.code : null;
	} catch {
		return null;
	}
}
