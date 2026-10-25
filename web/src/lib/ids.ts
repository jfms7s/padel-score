// Client ids for commands: a UUID per request, which makes a resend idempotent on the server.
//
// `crypto.randomUUID` only exists in a secure context (HTTPS or localhost). On plain HTTP it is
// missing and calling it throws, so the id is built from `crypto.getRandomValues`, which works
// everywhere, as an RFC 4122 version 4 UUID. Ids need to be unique, not secret, so with no
// crypto at all (never seen in a browser) Math.random stands in rather than throwing.

type CryptoLike = Partial<Pick<Crypto, 'randomUUID' | 'getRandomValues'>>;

const hex = (b: number) => b.toString(16).padStart(2, '0');

function randomBytes(c: CryptoLike | undefined): Uint8Array {
	const bytes = new Uint8Array(16);
	try {
		if (c?.getRandomValues) {
			c.getRandomValues(bytes);
			return bytes;
		}
	} catch {
		// Fall through to Math.random.
	}
	for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
	return bytes;
}

/** A fresh RFC 4122 v4 UUID. Never throws. */
export function newClientId(c: CryptoLike | undefined = globalThis.crypto): string {
	try {
		if (typeof c?.randomUUID === 'function') return c.randomUUID();
	} catch {
		// An insecure context: build one below.
	}
	const b = randomBytes(c);
	b[6] = (b[6]! & 0x0f) | 0x40; // version 4
	b[8] = (b[8]! & 0x3f) | 0x80; // variant 10
	const h = Array.from(b, hex).join('');
	return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}
