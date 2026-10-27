// Web push on this device: whether the browser can, turning it on and off, and a test.
// The server routes are crates/server/src/push_api.rs; the text of a push is #lib/notice.ts.

import { accountRequest } from './api';
import { safeStorage } from './storage';

export type PushSupport = 'unsupported' | 'ios-needs-home-screen' | 'denied' | 'available';

export interface PushEnv {
	hasServiceWorker: boolean;
	hasPushManager: boolean;
	hasNotification: boolean;
	/** `null` when the browser has no Notification API. */
	permission: NotificationPermission | null;
	isIos: boolean;
	/** Opened from the home screen, not a browser tab. */
	standalone: boolean;
}

/** What this browser can do, in the order the page explains it: unsupported, iPhone, blocked, ok. */
export function pushSupport(env: PushEnv): PushSupport {
	if (!env.hasServiceWorker || !env.hasNotification || env.permission === null) return 'unsupported';
	// iOS only has push for a web app added to the home screen, and exposes PushManager only there.
	if (env.isIos && !env.standalone) return 'ios-needs-home-screen';
	if (!env.hasPushManager) return 'unsupported';
	if (env.permission === 'denied') return 'denied';
	return 'available';
}

/** The environment of this browser; `pushSupport` of it is what the Account page shows. */
export function currentPushEnv(): PushEnv {
	const hasNotification = typeof Notification !== 'undefined';
	const nav = navigator as Navigator & { standalone?: boolean };
	return {
		hasServiceWorker: 'serviceWorker' in navigator,
		hasPushManager: 'PushManager' in window,
		hasNotification,
		permission: hasNotification ? Notification.permission : null,
		isIos:
			/iPad|iPhone|iPod/.test(navigator.userAgent) ||
			(navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1),
		standalone: window.matchMedia('(display-mode: standalone)').matches || nav.standalone === true
	};
}

/** A URL-safe base64 string (the VAPID public key) as bytes. */
export function urlBase64ToBytes(s: string): Uint8Array<ArrayBuffer> {
	const raw = atob(s.replace(/-/g, '+').replace(/_/g, '/'));
	const out = new Uint8Array(new ArrayBuffer(raw.length));
	for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
	return out;
}

const POST = 'POST' as const;
const DELETE = 'DELETE' as const;

async function subscription(): Promise<PushSubscription | null> {
	const reg = await navigator.serviceWorker.getRegistration();
	return (await reg?.pushManager.getSubscription()) ?? null;
}

/** Whether this device is subscribed in the browser. */
export async function devicePushOn(): Promise<boolean> {
	try {
		return (await subscription()) !== null;
	} catch {
		return false;
	}
}

// The endpoint this device last told the server about, so turning notifications off (or signing
// out) can still ask the server to drop it when the browser has already lost its subscription.
const ENDPOINT_KEY = 'push-endpoint';

function remember(endpoint: string | null): void {
	try {
		if (endpoint) safeStorage().setItem(ENDPOINT_KEY, endpoint);
		else safeStorage().removeItem(ENDPOINT_KEY);
	} catch {
		// Storage is only a convenience.
	}
}

function remembered(): string | null {
	try {
		return safeStorage().getItem(ENDPOINT_KEY);
	} catch {
		return null;
	}
}

async function tellServer(sub: PushSubscription): Promise<void> {
	await accountRequest<void>('/api/me/push', {
		method: POST,
		body: sub.toJSON(),
		ok: 204,
		signedIn: true
	});
	remember(sub.endpoint);
}

/**
 * For a signed-in page: if the browser holds a subscription, tell the server it is this account's
 * (an idempotent upsert that moves a device left by someone else to the signed-in user and
 * restores a row the server dropped). True only once the server has it, so "On" is never shown
 * for a device the server does not push to.
 */
export async function syncPush(): Promise<boolean> {
	try {
		const sub = await subscription();
		if (!sub) return false;
		await tellServer(sub);
		return true;
	} catch {
		return false;
	}
}

/**
 * Asks for permission, subscribes this device and tells the server. Any failure past the
 * permission prompt is `failed`; the browser's subscription is dropped so the switch stays off.
 */
export async function enablePush(): Promise<'on' | 'denied' | 'failed'> {
	let sub: PushSubscription | null = null;
	try {
		if ((await Notification.requestPermission()) !== 'granted') return 'denied';
		const reg = await navigator.serviceWorker.ready;
		const { key } = await accountRequest<{ key: string }>('/api/push/key');
		sub = await reg.pushManager.subscribe({
			userVisibleOnly: true,
			applicationServerKey: urlBase64ToBytes(key)
		});
		await tellServer(sub);
		return 'on';
	} catch {
		await sub?.unsubscribe().catch(() => false);
		return 'failed';
	}
}

/** Unsubscribes this device and removes it from the server. */
export async function disablePush(): Promise<void> {
	const sub = await subscription();
	const endpoint = sub?.endpoint ?? remembered();
	if (!endpoint) return;
	// The server first: if it fails the device stays subscribed and the switch stays on.
	await accountRequest<void>('/api/me/push', {
		method: DELETE,
		body: { endpoint },
		ok: 204,
		signedIn: true
	});
	remember(null);
	await sub?.unsubscribe();
}

/**
 * Before signing out: stop this device's pushes for the account that is leaving, while its
 * session can still ask the server. Best effort and never throws; if the server cannot be
 * reached the browser subscription is dropped anyway (a leftover server row dies on its first 410).
 */
export async function leavePush(): Promise<void> {
	try {
		await disablePush();
	} catch {
		remember(null);
		try {
			await (await subscription())?.unsubscribe();
		} catch {
			// Nothing more to do.
		}
	}
}

/** Asks the server to push a test notice to this account's devices. */
export async function sendTestPush(): Promise<void> {
	await accountRequest<unknown>('/api/me/push/test', { method: POST, ok: 202, signedIn: true });
}
