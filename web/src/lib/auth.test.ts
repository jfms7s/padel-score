import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, getScore, getView, postCommands, setUnauthorizedHandler } from './api';
import {
	acceptInvite,
	approveDevice,
	authMessage,
	changePassword,
	createInvite,
	createResetLink,
	denyDevice,
	disableUser,
	enableUser,
	formatUserCode,
	getMe,
	getSetup,
	initialLinkState,
	isCompleteUserCode,
	listDevices,
	listInvites,
	listMyMatches,
	listUsers,
	login,
	logout,
	lookupDevice,
	lookupInvite,
	resetPassword,
	revokeDevice,
	revokeInvite,
	setup,
	takeFragmentCode
} from './auth';
import {
	isAdminUser,
	isCreatedInvite,
	isDevice,
	isDeviceLookup,
	isInvite,
	isInviteLookup,
	isMe,
	isMyMatch,
	isResetLink,
	isSetupStatus
} from './guards';
import { strings } from './strings';
import meAdmin from './fixtures/me-admin.json';
import meUser from './fixtures/me-user.json';
import invitesList from './fixtures/invites-list.json';
import inviteCreated from './fixtures/invite-created.json';
import inviteLookup from './fixtures/invite-lookup.json';
import adminUsers from './fixtures/admin-users.json';
import resetLink from './fixtures/reset-link-created.json';
import myMatches from './fixtures/my-matches.json';
import setupNeeded from './fixtures/setup-status-needed.json';
import setupDone from './fixtures/setup-status-done.json';
import deviceLookup from './fixtures/device-lookup.json';
import devicesList from './fixtures/devices-list.json';
import deviceUsed from './fixtures/device-409-used.json';
import deviceNotFound from './fixtures/device-404-not-found.json';
import unauthorized from './fixtures/auth-401-unauthorized.json';
import invalidCredentials from './fixtures/login-401-invalid-credentials.json';
import forbidden from './fixtures/admin-403-forbidden.json';
import emailTaken from './fixtures/invite-accept-409-email-taken.json';
import setupDoneConflict from './fixtures/setup-409-done.json';
import inviteInvalid from './fixtures/invite-410-invalid.json';
import validation from './fixtures/setup-422-validation.json';
import tooMany from './fixtures/login-429-too-many.json';

const STRICT = { strict: true } as const;

type Call = { url: string; init: RequestInit | undefined };

function serve(status: number, body: unknown, headers: Record<string, string> = {}): Call[] {
	const calls: Call[] = [];
	vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
		calls.push({ url, init });
		const text = body === undefined ? null : JSON.stringify(body);
		return new Response(text, { status, headers });
	});
	return calls;
}

const fail = (p: Promise<unknown>) => p.then(() => null, (e: unknown) => e as ApiError);

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('the auth client', () => {
	it('auth_client_maps_status_codes', async () => {
		serve(401, invalidCredentials);
		const bad = await fail(login('a@b.test', 'x'));
		expect(bad).toMatchObject({ status: 401, code: 'invalid_credentials' });
		expect(authMessage(bad)).toBe(strings.auth.errors.invalid_credentials);

		serve(401, unauthorized);
		expect(await fail(getMe())).toMatchObject({ status: 401, code: 'unauthorized' });

		serve(403, forbidden);
		const denied = await fail(setup({ setup_code: 'x', email: 'a@b.test', display_name: 'A', password: '12345678' }));
		expect(denied).toMatchObject({ status: 403, code: 'forbidden' });
		expect(authMessage(denied)).toBe(strings.auth.errors.forbidden);

		serve(409, emailTaken);
		const taken = await fail(acceptInvite('code', { email: 'a@b.test', display_name: 'A', password: '12345678' }));
		expect(taken).toMatchObject({ status: 409, code: 'email_taken' });
		expect(authMessage(taken)).toBe(strings.auth.errors.email_taken);

		serve(409, setupDoneConflict);
		expect(await fail(setup({ setup_code: 'x', email: 'a@b.test', display_name: 'A', password: '12345678' }))).toMatchObject({
			status: 409,
			code: 'setup_done'
		});

		serve(409, deviceUsed);
		const used = await fail(approveDevice('ABCD-EFGH'));
		expect(used).toMatchObject({ status: 409, code: 'device_code_used' });
		expect(authMessage(used)).toBe(strings.auth.errors.device_code_used);
		serve(404, deviceNotFound);
		expect(await fail(lookupDevice('ZZZZ-ZZZZ'))).toMatchObject({ status: 404, code: 'not_found' });

		serve(410, inviteInvalid);
		const gone = await fail(lookupInvite('code'));
		expect(gone).toMatchObject({ status: 410, code: 'invite_invalid' });
		expect(authMessage(gone)).toBe(strings.auth.errors.invite_invalid);

		serve(422, validation);
		const invalid = await fail(setup({ setup_code: 'x', email: 'a@b.test', display_name: 'A', password: 'short' }));
		expect(invalid).toMatchObject({ status: 422, code: 'validation', field: 'password' });
		expect(authMessage(invalid)).toBe(strings.auth.errors.field.password);

		serve(429, tooMany, { 'retry-after': '58' });
		const limited = await fail(login('a@b.test', 'x'));
		expect(limited).toMatchObject({ status: 429, code: 'too_many_requests', retryAfterSecs: 58 });
		expect(authMessage(limited)).toBe('Too many attempts. Try again in 1 minute.');

		// The Retry-After header stands in for a body without the seconds.
		serve(429, { error: 'too_many_requests' }, { 'retry-after': '150' });
		const header = await fail(login('a@b.test', 'x'));
		expect(header).toMatchObject({ retryAfterSecs: 150 });
		expect(authMessage(header)).toBe('Too many attempts. Try again in 3 minutes.');

		// Unknown codes and network failures still read as something.
		serve(409, { error: 'something_new' });
		expect(authMessage(await fail(revokeInvite('i')))).toBe(strings.serverError(409));
		vi.stubGlobal('fetch', async () => {
			throw new TypeError('Failed to fetch');
		});
		expect(authMessage(await fail(listUsers()))).toBe(strings.networkError);
	});

	it('sends each route its method, path and body', async () => {
		const calls = serve(204, undefined);
		await logout();
		await changePassword('old-pass-1', 'new-pass-1');
		await revokeInvite('a/b');
		await disableUser('u1');
		await enableUser('u1');
		await resetPassword('code', 'new-pass-1');
		await approveDevice('ABCD-EFGH');
		await denyDevice('ABCD-EFGH');
		await revokeDevice('d1');
		expect(calls.map((c) => `${c.init?.method ?? 'GET'} ${c.url}`)).toEqual([
			'POST /api/auth/logout',
			'POST /api/auth/password',
			'DELETE /api/invites/a%2Fb',
			'POST /api/admin/users/u1/disable',
			'POST /api/admin/users/u1/enable',
			'POST /api/reset',
			'POST /api/device/ABCD-EFGH/approve',
			'POST /api/device/ABCD-EFGH/deny',
			'DELETE /api/me/devices/d1'
		]);
		expect(JSON.parse(String(calls[1]?.init?.body))).toEqual({ current_password: 'old-pass-1', new_password: 'new-pass-1' });
		expect(JSON.parse(String(calls[5]?.init?.body))).toEqual({ code: 'code', password: 'new-pass-1' });
	});

	it('sends invite codes in the body, never the path', async () => {
		const looked = serve(200, { invited_by_name: 'Ana', expires_at: 1 });
		await lookupInvite('se/cret');
		expect(looked.map((c) => `${c.init?.method ?? 'GET'} ${c.url}`)).toEqual(['POST /api/invites/lookup']);
		expect(JSON.parse(String(looked[0]?.init?.body))).toEqual({ code: 'se/cret' });

		const accepted = serve(201, meAdmin);
		const req = { email: 'a@b.test', display_name: 'A', password: '12345678' };
		await acceptInvite('se/cret', req);
		expect(accepted.map((c) => `${c.init?.method ?? 'GET'} ${c.url}`)).toEqual(['POST /api/invites/accept']);
		expect(JSON.parse(String(accepted[0]?.init?.body))).toEqual({ code: 'se/cret', ...req });
	});

	it('reads each shape the server sends', async () => {
		serve(200, setupNeeded);
		expect(await getSetup()).toEqual({ needs_setup: true });
		serve(200, meAdmin);
		expect(await login('ana@example.test', 'pw')).toEqual(meAdmin);
		serve(201, inviteCreated);
		expect(await createInvite()).toEqual(inviteCreated);
		serve(200, invitesList);
		expect(await listInvites()).toEqual(invitesList);
		serve(200, adminUsers);
		expect(await listUsers()).toEqual(adminUsers);
		serve(201, resetLink);
		expect(await createResetLink('u1')).toEqual(resetLink);
		serve(200, myMatches);
		expect(await listMyMatches()).toEqual(myMatches);
		const calls = serve(200, deviceLookup);
		expect(await lookupDevice('ABCD-EFGH')).toEqual(deviceLookup);
		expect(calls[0]?.url).toBe('/api/device/ABCD-EFGH');
		serve(200, devicesList);
		expect(await listDevices()).toEqual(devicesList);
	});

	it('refuses an answer of the wrong shape', async () => {
		serve(200, [{ id: 'x' }]);
		await expect(listInvites()).rejects.toMatchObject({ code: 'internal' });
		serve(200, { device_name: 3 });
		await expect(lookupDevice('ABCD-EFGH')).rejects.toMatchObject({ code: 'internal' });
	});
});

describe('the global 401 handler', () => {
	it('global_401_redirects_only_for_signed_in_calls', async () => {
		const redirect = vi.fn();
		const restore = setUnauthorizedHandler(redirect);
		try {
			serve(401, unauthorized);
			// Signed-in calls: an expired or revoked session goes to sign-in.
			await fail(listInvites());
			await fail(listMyMatches());
			await fail(listUsers());
			await fail(lookupDevice('ABCD-EFGH'));
			await fail(changePassword('a', 'b'));
			expect(redirect).toHaveBeenCalledTimes(5);
			redirect.mockClear();

			// The quiet boot check, the sign-in routes and the public code routes never redirect.
			await fail(getMe());
			await fail(logout());
			await fail(lookupInvite('c'));
			await fail(acceptInvite('c', { email: 'a@b', display_name: 'A', password: '12345678' }));
			await fail(resetPassword('c', '12345678'));
			// Nor do the anonymous scorer and spectator routes.
			await fail(getScore('tok'));
			await fail(getView('tok'));
			await fail(postCommands('tok', { client_id: 'c', batch: false, base_revision: 0, commands: [] }));
			expect(redirect).not.toHaveBeenCalled();

			// A wrong password is an answer, not a lost session.
			serve(401, invalidCredentials);
			await fail(changePassword('wrong', 'new-pass-1'));
			await fail(login('a@b.test', 'wrong'));
			expect(redirect).not.toHaveBeenCalled();
		} finally {
			restore();
		}
	});

	it('the default handler sends the browser to sign-in with the path as next', async () => {
		const assign = vi.fn();
		vi.stubGlobal('location', { pathname: '/account', search: '?tab=1', hash: '#secret', assign });
		serve(401, unauthorized);
		await fail(listInvites());
		expect(assign).toHaveBeenCalledWith('/login?next=%2Faccount%3Ftab%3D1');
	});

	it('the default handler stays put on the sign-in page', async () => {
		const assign = vi.fn();
		vi.stubGlobal('location', { pathname: '/login', search: '', hash: '', assign });
		serve(401, unauthorized);
		await fail(listInvites());
		expect(assign).not.toHaveBeenCalled();
	});
});

describe('codes in the page', () => {
	function fakePage(hash: string) {
		const loc = { pathname: '/invite', search: '', hash };
		const replaceState = vi.fn((_state: unknown, _unused: string, url: string) => {
			loc.hash = url.includes('#') ? url.slice(url.indexOf('#')) : '';
		});
		return { loc, hist: { state: { kit: 1 }, replaceState } };
	}

	it('invite_code_read_from_fragment_then_cleared', () => {
		const { loc, hist } = fakePage('#HwnJKxrrVConylRitdWByPr5GaZ3nRNny2CfF8hJtZg');
		expect(takeFragmentCode(loc, hist)).toBe('HwnJKxrrVConylRitdWByPr5GaZ3nRNny2CfF8hJtZg');
		expect(hist.replaceState).toHaveBeenCalledWith({ kit: 1 }, '', '/invite');
		expect(loc.hash).toBe('');
	});

	it('an empty fragment is no code, and a percent-encoded one is decoded', () => {
		expect(takeFragmentCode(fakePage('').loc, fakePage('').hist)).toBeNull();
		expect(takeFragmentCode(fakePage('#').loc, fakePage('#').hist)).toBeNull();
		const p = fakePage('#a%2Db');
		expect(takeFragmentCode(p.loc, p.hist)).toBe('a-b');
		const broken = fakePage('#%E0%A4%A');
		expect(takeFragmentCode(broken.loc, broken.hist)).toBeNull();
		expect(broken.hist.replaceState).toHaveBeenCalled();
	});

	it('link_page_never_prefills_code', () => {
		vi.stubGlobal('location', {
			pathname: '/link',
			search: '?code=ABCD-EFGH&user_code=ABCD-EFGH',
			hash: '#ABCDEFGH',
			href: 'https://padel.example/link?code=ABCD-EFGH#ABCDEFGH'
		});
		expect(initialLinkState()).toEqual({ step: 'enter', code: '' });
	});

	it('user_code_formatting', () => {
		expect(formatUserCode('')).toBe('');
		expect(formatUserCode('ab')).toBe('AB');
		expect(formatUserCode('abcd')).toBe('ABCD');
		expect(formatUserCode('abcde')).toBe('ABCD-E');
		expect(formatUserCode('abcd-efgh')).toBe('ABCD-EFGH');
		expect(formatUserCode(' ab cd ef gh ')).toBe('ABCD-EFGH');
		expect(formatUserCode('ABCD-')).toBe('ABCD');
		expect(formatUserCode('abcdefghjk')).toBe('ABCD-EFGH');
		expect(formatUserCode('a.b_c/d')).toBe('ABCD');
		expect(isCompleteUserCode('ABCD-EFGH')).toBe(true);
		expect(isCompleteUserCode('ABCD-EFG')).toBe(false);
	});
});

describe('account guards against the recorded answers', () => {
	it('Me', () => {
		expect(isMe(meAdmin, STRICT)).toBe(true);
		expect(isMe(meUser, STRICT)).toBe(true);
		expect(isMe({ ...meAdmin, extra: 1 }, STRICT)).toBe(false);
		expect(isMe({ ...meAdmin, extra: 1 })).toBe(true);
		expect(isMe({ ...meAdmin, is_admin: 'yes' })).toBe(false);
	});

	it('Invite', () => {
		expect(invitesList.length).toBe(3);
		for (const i of invitesList) expect(isInvite(i, STRICT)).toBe(true);
		expect(isInvite({ ...invitesList[0], status: 'lost' })).toBe(false);
		expect(isInvite({ ...invitesList[2], used_by_name: 3 })).toBe(false);
		expect(isCreatedInvite(inviteCreated, STRICT)).toBe(true);
		expect(isInviteLookup(inviteLookup, STRICT)).toBe(true);
		expect(isResetLink(resetLink, STRICT)).toBe(true);
	});

	it('Device', () => {
		expect(isDeviceLookup(deviceLookup, STRICT)).toBe(true);
		for (const d of devicesList) expect(isDevice(d, STRICT)).toBe(true);
		expect(devicesList.some((d) => 'last_used_at' in d)).toBe(true);
		expect(devicesList.some((d) => !('last_used_at' in d))).toBe(true);
		expect(isDevice({ ...devicesList[0], last_used_at: 'soon' })).toBe(false);
		expect(isDevice({ ...devicesList[0], last_used_at: null })).toBe(true);
	});

	it('MyMatch', () => {
		expect(myMatches.length).toBeGreaterThan(0);
		for (const m of myMatches) expect(isMyMatch(m, STRICT)).toBe(true);
		const { scorer_token: _s, spectator_token: _v, ...device } = myMatches[0]!;
		expect(isMyMatch(device, STRICT)).toBe(true);
		expect(isMyMatch({ ...myMatches[0], view: {} })).toBe(false);
	});

	it('the admin list and setup status', () => {
		for (const u of adminUsers) expect(isAdminUser(u, STRICT)).toBe(true);
		expect(isSetupStatus(setupNeeded, STRICT)).toBe(true);
		expect(isSetupStatus(setupDone, STRICT)).toBe(true);
	});
});
