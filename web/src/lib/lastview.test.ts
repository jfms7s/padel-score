import { describe, expect, it } from 'vitest';
import { loadView, saveView } from './lastview';
import { asView } from './fixtures/load';
import onePoint from './fixtures/view-one-point.json';
import newMatch from './fixtures/view-new.json';

function store(init: Record<string, string> = {}) {
	const m = new Map(Object.entries(init));
	return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), m };
}

describe('last view', () => {
	it('cold_offline_open_uses_stored_view', () => {
		const s = store();
		const v = asView(onePoint);
		saveView(s, 'tok', v);
		expect(loadView(s, 'tok')).toEqual(v);
		expect(loadView(s, 'other')).toBeNull();
		expect(s.m.has('padel-score:view:tok')).toBe(true);
	});
	it('corrupt_stored_view_is_ignored', () => {
		expect(loadView(store({ 'padel-score:view:t': '{not json' }), 't')).toBeNull();
		expect(loadView(store({ 'padel-score:view:t': '{"revision":1}' }), 't')).toBeNull();
	});
	it('older_revision_never_overwrites_stored_view', () => {
		const s = store();
		const newer = { ...asView(onePoint), revision: 9 };
		const older = { ...asView(newMatch), revision: 2 };
		saveView(s, 't', newer);
		saveView(s, 't', older);
		expect(loadView(s, 't')?.revision).toBe(9);
	});
	it('a storage that throws is tolerated', () => {
		const bad = { getItem: () => { throw new Error('x'); }, setItem: () => { throw new Error('x'); } };
		expect(() => saveView(bad, 't', asView(onePoint))).not.toThrow();
		expect(loadView(bad, 't')).toBeNull();
	});
});
