import { describe, expect, it } from 'vitest';
import { noticeText, safeNoticeUrl } from './notice';

describe('noticeText', () => {
	it('says notifications are on for the test kind', () => {
		const n = noticeText({ kind: 'test', url: '/account' });
		expect(n.title).toBe('Notifications are on');
		expect(n.body).toBe("You'll get game and event updates on this device.");
		expect(n.url).toBe('/account');
		expect(n.tag).toBe('test:/account');
	});

	it('falls back for a kind this build does not know', () => {
		const n = noticeText({ kind: 'from_the_future', url: '/b/abc' });
		expect(n.title).toBe('Padel score');
		expect(n.body).toBe('Something changed — tap to see');
		expect(n.url).toBe('/b/abc');
		expect(n.tag).toBe('from_the_future:/b/abc');
	});

	it('opens the calendar when the payload has no usable url', () => {
		expect(noticeText({ kind: 'test' }).url).toBe('/calendar');
		expect(noticeText({ kind: 'test', url: 'https://evil.test/x' }).tag).toBe('test:/calendar');
	});
});

describe('safeNoticeUrl', () => {
	it('notice_url_must_be_a_same_origin_path', () => {
		for (const bad of [
			'https://evil.test/x',
			'//evil.test/x',
			'/\\evil.test',
			'javascript:alert(1)',
			42,
			undefined,
			null,
			'',
			'/\t/evil.test'
		]) {
			expect(safeNoticeUrl(bad)).toBe('/calendar');
		}
		expect(safeNoticeUrl('/b/abc')).toBe('/b/abc');
		expect(safeNoticeUrl('/j/abc?x=1')).toBe('/j/abc?x=1');
	});
});
