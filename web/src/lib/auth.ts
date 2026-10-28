// The account routes (crates/server/src/accounts.rs), the device approval routes (device.rs)
// and the owned matches (owned.rs), typed, plus the small helpers their pages share.
//
// Every call goes through api.ts's accountRequest. `signedIn` marks the calls that need a
// session: on those a 401 means it ended, and the browser goes to sign-in. The boot check
// (getMe), sign-in itself and the public code routes (an invite, a reset) stay quiet.

import { accountRequest, ApiError } from './api';
import { dateText } from './dates';
import {
	isAdminUser,
	isCreatedInvite,
	isDevice,
	isDeviceLookup,
	isInvite,
	isInviteLookup,
	isListOf,
	isMe,
	isMyMatch,
	isResetLink,
	isSetupStatus
} from './guards';
import { strings } from './strings';
import type {
	AcceptInviteRequest,
	AdminUser,
	CreatedInvite,
	Device,
	DeviceLookup,
	Invite,
	InviteLookup,
	Me,
	MyMatch,
	ResetLink,
	SetupRequest,
	SetupStatus
} from './types';

const enc = encodeURIComponent;
const POST = 'POST' as const;
const DELETE = 'DELETE' as const;

// ---- setup and sign-in -------------------------------------------------------------------

export function getSetup(): Promise<SetupStatus> {
	return accountRequest('/api/setup', { guard: isSetupStatus });
}

/** Creates the first admin, signed in. */
export function setup(req: SetupRequest): Promise<Me> {
	return accountRequest('/api/setup', { method: POST, body: req, ok: 201, guard: isMe });
}

export function login(email: string, password: string): Promise<Me> {
	return accountRequest('/api/auth/login', { method: POST, body: { email, password }, guard: isMe });
}

export function logout(): Promise<void> {
	return accountRequest('/api/auth/logout', { method: POST, ok: 204 });
}

/** Who is signed in; a 401 means nobody, and is not a redirect. */
export function getMe(): Promise<Me> {
	return accountRequest('/api/auth/me', { guard: isMe });
}

/** Signs out every other session and device. */
export function changePassword(current: string, next: string): Promise<void> {
	return accountRequest('/api/auth/password', {
		method: POST,
		body: { current_password: current, new_password: next },
		ok: 204,
		signedIn: true
	});
}

// ---- invites -----------------------------------------------------------------------------

export function createInvite(): Promise<CreatedInvite> {
	return accountRequest('/api/invites', { method: POST, ok: 201, signedIn: true, guard: isCreatedInvite });
}

export function listInvites(): Promise<Invite[]> {
	return accountRequest('/api/invites', { signedIn: true, guard: isListOf(isInvite) });
}

export function revokeInvite(id: string): Promise<void> {
	return accountRequest(`/api/invites/${enc(id)}`, { method: DELETE, ok: 204, signedIn: true });
}

/** Public: who sent the invite. The code goes in the body, never a path a log could keep. */
export function lookupInvite(code: string): Promise<InviteLookup> {
	return accountRequest('/api/invites/lookup', { method: POST, body: { code }, guard: isInviteLookup });
}

/** Public: creates the account, signed in. The code goes in the body, as for the lookup. */
export function acceptInvite(code: string, req: AcceptInviteRequest): Promise<Me> {
	return accountRequest('/api/invites/accept', { method: POST, body: { code, ...req }, ok: 201, guard: isMe });
}

// ---- admin and reset ---------------------------------------------------------------------

export function listUsers(): Promise<AdminUser[]> {
	return accountRequest('/api/admin/users', { signedIn: true, guard: isListOf(isAdminUser) });
}

export function disableUser(id: string): Promise<void> {
	return accountRequest(`/api/admin/users/${enc(id)}/disable`, { method: POST, ok: 204, signedIn: true });
}

export function enableUser(id: string): Promise<void> {
	return accountRequest(`/api/admin/users/${enc(id)}/enable`, { method: POST, ok: 204, signedIn: true });
}

export function createResetLink(id: string): Promise<ResetLink> {
	return accountRequest(`/api/admin/users/${enc(id)}/reset-link`, {
		method: POST,
		ok: 201,
		signedIn: true,
		guard: isResetLink
	});
}

/** Public: sets the password and signs the user out everywhere; nobody is signed in after. */
export function resetPassword(code: string, password: string): Promise<void> {
	return accountRequest('/api/reset', { method: POST, body: { code, password }, ok: 204 });
}

// ---- watches -----------------------------------------------------------------------------

export function lookupDevice(userCode: string): Promise<DeviceLookup> {
	return accountRequest(`/api/device/${enc(userCode)}`, { signedIn: true, guard: isDeviceLookup });
}

export function approveDevice(userCode: string): Promise<void> {
	return accountRequest(`/api/device/${enc(userCode)}/approve`, { method: POST, ok: 204, signedIn: true });
}

export function denyDevice(userCode: string): Promise<void> {
	return accountRequest(`/api/device/${enc(userCode)}/deny`, { method: POST, ok: 204, signedIn: true });
}

export function listDevices(): Promise<Device[]> {
	return accountRequest('/api/me/devices', { signedIn: true, guard: isListOf(isDevice) });
}

export function revokeDevice(id: string): Promise<void> {
	return accountRequest(`/api/me/devices/${enc(id)}`, { method: DELETE, ok: 204, signedIn: true });
}

// ---- owned matches -----------------------------------------------------------------------

export function listMyMatches(): Promise<MyMatch[]> {
	return accountRequest('/api/me/matches', { signedIn: true, guard: isListOf(isMyMatch) });
}

// ---- messages ----------------------------------------------------------------------------

const MESSAGES: Record<string, string> = {
	invalid_credentials: strings.auth.errors.invalid_credentials,
	unauthorized: strings.auth.errors.unauthorized,
	forbidden: strings.auth.errors.forbidden,
	setup_done: strings.auth.errors.setup_done,
	email_taken: strings.auth.errors.email_taken,
	too_many_invites: strings.auth.errors.too_many_invites,
	invite_used: strings.auth.errors.invite_used,
	invite_invalid: strings.auth.errors.invite_invalid,
	reset_invalid: strings.auth.errors.reset_invalid,
	cannot_disable_self: strings.auth.errors.cannot_disable_self,
	last_admin: strings.auth.errors.last_admin,
	device_code_used: strings.auth.errors.device_code_used,
	not_found: strings.auth.errors.not_found
};

const FIELDS: Record<string, string> = strings.auth.errors.field;

/** What to tell the user about a failed account call. */
export function authMessage(e: unknown): string {
	if (!(e instanceof ApiError)) return strings.networkError;
	if (e.code === 'network') return strings.networkError;
	if (e.code === 'too_many_requests') {
		return strings.auth.errors.tooMany(Math.max(1, Math.ceil((e.retryAfterSecs ?? 60) / 60)));
	}
	if (e.code === 'validation') return FIELDS[e.field ?? ''] ?? FIELDS.other!;
	return MESSAGES[e.code] ?? strings.serverError(e.status);
}

/** A server time (Unix seconds) as a short local date. */
export function shortDate(secs: number): string {
	return dateText(secs);
}

// ---- codes in the page -------------------------------------------------------------------

interface PageLocation {
	pathname: string;
	search: string;
	hash: string;
}
interface PageHistory {
	state: unknown;
	replaceState(state: unknown, unused: string, url: string): void;
}

/**
 * The secret code in the URL fragment (`/invite#<code>`, `/reset#<code>`), which is then removed
 * from the address bar and the history entry at once, so it can't be shared on, bookmarked or
 * read back. Null when there is none, or it can't be decoded.
 */
export function takeFragmentCode(loc: PageLocation, hist: PageHistory): string | null {
	const raw = loc.hash.startsWith('#') ? loc.hash.slice(1) : loc.hash;
	if (loc.hash !== '') hist.replaceState(hist.state, '', loc.pathname + loc.search);
	if (raw === '') return null;
	try {
		return decodeURIComponent(raw);
	} catch {
		return null;
	}
}

/** How many characters a watch code has, without the dash. */
export const USER_CODE_LENGTH = 8;

/** A watch code as typed: letters and digits only, uppercased, at most 8, as `XXXX-XXXX`. */
export function formatUserCode(input: string): string {
	const chars = input
		.toUpperCase()
		.replace(/[^A-Z0-9]/g, '')
		.slice(0, USER_CODE_LENGTH);
	return chars.length > 4 ? `${chars.slice(0, 4)}-${chars.slice(4)}` : chars;
}

export function isCompleteUserCode(code: string): boolean {
	return /^[A-Z0-9]{4}-[A-Z0-9]{4}$/.test(code);
}

export type LinkState =
	| { step: 'enter'; code: string }
	| { step: 'confirm'; code: string; device: DeviceLookup }
	| { step: 'done'; approved: boolean };

/**
 * Where /link starts. Never from the URL: a code arriving in a link could be an attacker's own
 * watch, so the person must read it off their watch and type it.
 */
export function initialLinkState(): LinkState {
	return { step: 'enter', code: '' };
}
