import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	claimGuest,
	forgetGuest,
	GUEST_KEY_PREFIX,
	guestKey,
	joinAsGuest,
	pickGuest,
	rememberedGuestId,
	rememberGuest,
	removeGuest
} from './guests';
import type { StorageLike } from './setup';

function memory(): StorageLike & { data: Map<string, string> } {
	const data = new Map<string, string>();
	return {
		data,
		getItem: (k) => data.get(k) ?? null,
		setItem: (k, v) => void data.set(k, v),
		removeItem: (k) => void data.delete(k)
	};
}

type Call = { url: string; init: RequestInit | undefined };

function serve(status: number, body: unknown): Call[] {
	const calls: Call[] = [];
	vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
		calls.push({ url, init });
		return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
	});
	return calls;
}

const joined = (id: string | null) => ({ guest_key: 'k1', view: { me: id === null ? null : { user_id: id } } });

afterEach(() => vi.unstubAllGlobals());

describe('remembered guest', () => {
	it('remembers, reads and forgets the key and id under the board token', () => {
		const s = memory();
		expect(guestKey('t', s)).toBeNull();
		rememberGuest('t', 'k', 'guest_1', s);
		expect(s.data.get(`${GUEST_KEY_PREFIX}t`)).toBe('{"key":"k","id":"guest_1"}');
		expect(guestKey('t', s)).toBe('k');
		expect(rememberedGuestId('t', s)).toBe('guest_1');
		expect(guestKey('other', s)).toBeNull();
		forgetGuest('t', s);
		expect(guestKey('t', s)).toBeNull();
		expect(rememberedGuestId('t', s)).toBeNull();
	});

	it('reads an unreadable value as nothing', () => {
		const s = memory();
		s.setItem(`${GUEST_KEY_PREFIX}t`, 'not json');
		expect(guestKey('t', s)).toBeNull();
		expect(rememberedGuestId('t', s)).toBeNull();
		s.setItem(`${GUEST_KEY_PREFIX}t`, '{"key":1}');
		expect(guestKey('t', s)).toBeNull();
	});
});

describe('guest calls', () => {
	it('joinAsGuest posts the name and keeps the key and id', async () => {
		const s = memory();
		vi.stubGlobal('localStorage', s);
		const calls = serve(201, joined('guest_9'));
		const out = await joinAsGuest('a b', 'Ana');
		expect(out.guest_key).toBe('k1');
		expect(calls[0]?.url).toBe('/api/boards/a%20b/guests');
		expect(calls[0]?.init?.method).toBe('POST');
		expect(calls[0]?.init?.body).toBe('{"name":"Ana"}');
		expect(guestKey('a b', s)).toBe('k1');
		expect(rememberedGuestId('a b', s)).toBe('guest_9');
	});

	it('pickGuest posts to the guest and keeps the key', async () => {
		const s = memory();
		vi.stubGlobal('localStorage', s);
		const calls = serve(200, joined('guest_2'));
		await pickGuest('t', 'guest_2');
		expect(calls[0]?.url).toBe('/api/boards/t/guests/guest_2/pick');
		expect(calls[0]?.init?.method).toBe('POST');
		expect(guestKey('t', s)).toBe('k1');
	});

	it('stores nothing when the view has no me', async () => {
		const s = memory();
		vi.stubGlobal('localStorage', s);
		serve(201, joined(null));
		await expect(joinAsGuest('t', 'Ana')).rejects.toBeInstanceOf(Error);
		expect(guestKey('t', s)).toBeNull();
	});

	it('removeGuest deletes the guest', async () => {
		const calls = serve(200, {});
		await removeGuest('t', 'guest_2');
		expect(calls[0]?.url).toBe('/api/boards/t/guests/guest_2');
		expect(calls[0]?.init?.method).toBe('DELETE');
	});

	it('claimGuest posts the claim and forgets the key', async () => {
		const s = memory();
		vi.stubGlobal('localStorage', s);
		rememberGuest('t', 'k', 'guest_2', s);
		const calls = serve(200, {});
		await claimGuest('t', 'guest_2');
		expect(calls[0]?.url).toBe('/api/boards/t/guests/guest_2/claim');
		expect(calls[0]?.init?.method).toBe('POST');
		expect(guestKey('t', s)).toBeNull();
	});
});
