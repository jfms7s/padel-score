import { describe, expect, it } from 'vitest';
import { loginPath, safeNext } from './next';

describe('the next path guard', () => {
	it('next_param_rejects_external_and_protocol_relative', () => {
		for (const bad of [
			'https://evil.example/',
			'http:/evil.example',
			'javascript:alert(1)',
			'//evil.example/path',
			'/\\evil.example',
			'\\\\evil.example',
			'/\t/evil.example',
			'/\n/evil.example',
			' /account',
			'account',
			'',
			null,
			undefined,
			'/'.padEnd(3000, 'a')
		]) {
			expect(safeNext(bad), String(bad)).toBe('/');
		}
	});

	it('keeps same-site paths with their query', () => {
		expect(safeNext('/account')).toBe('/account');
		expect(safeNext('/link?x=1')).toBe('/link?x=1');
		expect(safeNext('/s/abc')).toBe('/s/abc');
	});

	it('drops a fragment, which can hold a secret code', () => {
		expect(safeNext('/invite#code')).toBe('/invite');
	});

	it('never sends a sign-in back to the sign-in page', () => {
		expect(safeNext('/login?next=/account')).toBe('/');
	});

	it('builds the login path, encoding next', () => {
		expect(loginPath('/account')).toBe('/login?next=%2Faccount');
		expect(loginPath('/')).toBe('/login');
		expect(loginPath('//evil.example')).toBe('/login');
	});
});
