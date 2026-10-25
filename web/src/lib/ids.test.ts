import { describe, expect, it } from 'vitest';
import { newClientId } from './ids';

const V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

describe('newClientId', () => {
	it('uses randomUUID when there is one', () => {
		const id = newClientId({
			randomUUID: () => '00000000-0000-4000-8000-000000000000',
			getRandomValues: () => {
				throw new Error('unused');
			}
		});
		expect(id).toBe('00000000-0000-4000-8000-000000000000');
	});

	it('client_id_falls_back_without_randomUUID', () => {
		// As on plain HTTP: crypto.randomUUID is missing (an insecure context), getRandomValues is not.
		const insecure = { getRandomValues: <T extends ArrayBufferView>(a: T) => crypto.getRandomValues(a as never) as T };
		const ids = new Set<string>();
		for (let i = 0; i < 200; i++) {
			const id = newClientId(insecure);
			expect(id).toMatch(V4);
			ids.add(id);
		}
		expect(ids.size).toBe(200);
	});

	it('falls back when randomUUID throws', () => {
		const id = newClientId({
			randomUUID: () => {
				throw new TypeError('crypto.randomUUID is not a function');
			},
			getRandomValues: <T extends ArrayBufferView>(a: T) => crypto.getRandomValues(a as never) as T
		});
		expect(id).toMatch(V4);
	});
});
