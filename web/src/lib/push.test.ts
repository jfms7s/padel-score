import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('./api', () => ({ accountRequest: vi.fn() }));

import { accountRequest } from './api';
import { disablePush, enablePush, pushSupport, sendTestPush, urlBase64ToBytes, type PushEnv } from './push';

const env = (over: Partial<PushEnv> = {}): PushEnv => ({
	hasServiceWorker: true,
	hasPushManager: true,
	hasNotification: true,
	permission: 'default',
	isIos: false,
	standalone: false,
	...over
});

describe('urlBase64ToBytes', () => {
	it('round-trips a 65-byte key', () => {
		const bytes = Uint8Array.from({ length: 65 }, (_, i) => (i * 7 + 250) % 256);
		const b64 = btoa(String.fromCharCode(...bytes))
			.replace(/\+/g, '-')
			.replace(/\//g, '_')
			.replace(/=+$/, '');
		expect(Array.from(urlBase64ToBytes(b64))).toEqual(Array.from(bytes));
	});
});

describe('pushSupport', () => {
	it('is available in a normal browser, asked or already granted', () => {
		expect(pushSupport(env())).toBe('available');
		expect(pushSupport(env({ permission: 'granted' }))).toBe('available');
	});
	it('asks iPhone users for the home screen first', () => {
		expect(pushSupport(env({ isIos: true, hasPushManager: false }))).toBe('ios-needs-home-screen');
		expect(pushSupport(env({ isIos: true, standalone: false }))).toBe('ios-needs-home-screen');
	});
	it('is available from the home screen on iPhone with PushManager', () => {
		expect(pushSupport(env({ isIos: true, standalone: true }))).toBe('available');
	});
	it('is denied when permission is denied', () => {
		expect(pushSupport(env({ permission: 'denied' }))).toBe('denied');
	});
	it('is unsupported without a service worker, a PushManager or Notification', () => {
		expect(pushSupport(env({ hasServiceWorker: false }))).toBe('unsupported');
		expect(pushSupport(env({ hasPushManager: false }))).toBe('unsupported');
		expect(pushSupport(env({ hasNotification: false, permission: null }))).toBe('unsupported');
		expect(pushSupport(env({ permission: null }))).toBe('unsupported');
	});
	it('puts unsupported before the iPhone note, and the iPhone note before denied', () => {
		expect(pushSupport(env({ hasServiceWorker: false, isIos: true }))).toBe('unsupported');
		expect(pushSupport(env({ isIos: true, permission: 'denied' }))).toBe('ios-needs-home-screen');
	});
});

describe('the device calls', () => {
	const request = vi.mocked(accountRequest);
	afterEach(() => {
		vi.unstubAllGlobals();
		request.mockReset();
	});

	function browser(permission: NotificationPermission, sub: object | null = null) {
		const subscribe = vi.fn().mockResolvedValue(sub);
		const existing = sub as { unsubscribe: () => Promise<boolean>; endpoint: string } | null;
		vi.stubGlobal('Notification', { requestPermission: vi.fn().mockResolvedValue(permission) });
		vi.stubGlobal('navigator', {
			serviceWorker: {
				ready: Promise.resolve({ pushManager: { subscribe } }),
				getRegistration: () => Promise.resolve({ pushManager: { getSubscription: () => Promise.resolve(existing) } })
			}
		});
		return subscribe;
	}

	it('subscribes with the server key and posts the subscription', async () => {
		const json = { endpoint: 'https://push.example.test/a', keys: { p256dh: 'p', auth: 'a' } };
		const subscribe = browser('granted', { toJSON: () => json });
		request.mockResolvedValueOnce({ key: 'AQID' }).mockResolvedValueOnce(undefined);
		expect(await enablePush()).toBe('on');
		const arg = subscribe.mock.calls[0]?.[0];
		expect(arg.userVisibleOnly).toBe(true);
		expect(Array.from(arg.applicationServerKey)).toEqual([1, 2, 3]);
		expect(request).toHaveBeenLastCalledWith('/api/me/push', { method: 'POST', body: json, ok: 204, signedIn: true });
	});

	it('is denied, and asks the server nothing, when the user says no', async () => {
		browser('denied');
		expect(await enablePush()).toBe('denied');
		expect(request).not.toHaveBeenCalled();
	});

	it('fails, and drops the subscription, when the server refuses it', async () => {
		const unsubscribe = vi.fn().mockResolvedValue(true);
		browser('granted', { toJSON: () => ({}), unsubscribe });
		request.mockResolvedValueOnce({ key: 'AQID' }).mockRejectedValueOnce(new Error('422'));
		expect(await enablePush()).toBe('failed');
		expect(unsubscribe).toHaveBeenCalled();
	});

	it('fails when the key cannot be fetched', async () => {
		browser('granted');
		request.mockRejectedValueOnce(new Error('offline'));
		expect(await enablePush()).toBe('failed');
	});

	it('removes the subscription from the server, then the browser', async () => {
		const unsubscribe = vi.fn().mockResolvedValue(true);
		browser('granted', { endpoint: 'https://push.example.test/a', unsubscribe });
		request.mockResolvedValueOnce(undefined);
		await disablePush();
		expect(request).toHaveBeenCalledWith('/api/me/push', {
			method: 'DELETE',
			body: { endpoint: 'https://push.example.test/a' },
			ok: 204,
			signedIn: true
		});
		expect(unsubscribe).toHaveBeenCalled();
	});

	it('keeps the browser subscription when the server could not remove it', async () => {
		const unsubscribe = vi.fn();
		browser('granted', { endpoint: 'https://push.example.test/a', unsubscribe });
		request.mockRejectedValueOnce(new Error('offline'));
		await expect(disablePush()).rejects.toThrow();
		expect(unsubscribe).not.toHaveBeenCalled();
	});

	it('asks for a test notice', async () => {
		request.mockResolvedValueOnce({});
		await sendTestPush();
		expect(request).toHaveBeenCalledWith('/api/me/push/test', { method: 'POST', ok: 202, signedIn: true });
	});
});
