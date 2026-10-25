// Where to go after signing in: `/login?next=<path>`. Only a path on this site is kept, so a
// crafted sign-in link can't send someone elsewhere once they have typed their password.

const HOME = '/';
const MAX_NEXT = 2048;
/** Any base works: a path that stays on it is same-site, whatever the real origin. */
const BASE = 'https://padel-score.invalid';

/**
 * `raw` when it is a same-site path, else `/`. It must start with one `/` (not `//` or `/\`,
 * which browsers read as another host), and hold no whitespace, control character or backslash
 * (browsers strip or rewrite those, so `/\t/evil` turns into `//evil`). The fragment is dropped:
 * it can carry a secret code. Sign-in itself is never a destination.
 */
export function safeNext(raw: string | null | undefined): string {
	if (typeof raw !== 'string' || raw.length > MAX_NEXT) return HOME;
	if (!raw.startsWith('/') || raw.startsWith('//')) return HOME;
	if (/[\s\\\u0000-\u001f\u007f]/.test(raw)) return HOME;
	let url: URL;
	try {
		url = new URL(raw, BASE);
	} catch {
		return HOME;
	}
	if (url.origin !== BASE) return HOME;
	if (url.pathname === '/login') return HOME;
	return url.pathname + url.search;
}

/** The sign-in page, coming back to `next` (when it is worth keeping) afterwards. */
export function loginPath(next: string): string {
	const target = safeNext(next);
	return target === HOME ? '/login' : `/login?next=${encodeURIComponent(target)}`;
}
