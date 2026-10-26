<!--
	Owned by phase 8 (the non-stop). The non-stop's own settings on the new-event form: minutes per
	match, legs and the order of matches. Sends all three, with defaults for any the form hasn't set.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import { strings } from '#lib/strings.ts';

	const t = strings.nonStop;
	const LEGS = [1, 2, 3] as const;
	const ORDERS = ['listed', 'random'] as const;
	type Order = (typeof ORDERS)[number];

	let {
		settings,
		onchange
	}: {
		settings: Record<string, unknown>;
		onchange: (settings: Record<string, unknown>) => void;
	} = $props();

	const num = (v: unknown, d: number) => (typeof v === 'number' && Number.isFinite(v) ? v : d);
	const minutes = $derived(num(settings.match_minutes, 15));
	const legs = $derived(num(settings.legs, 1));
	const order = $derived<Order>(settings.order === 'random' ? 'random' : 'listed');

	function emit(change: Partial<{ match_minutes: number; legs: number; order: Order }>) {
		onchange({ ...settings, match_minutes: minutes, legs, order, ...change });
	}

	// An empty form still sends the defaults, so the event is created with them.
	$effect(() => {
		if (settings.match_minutes === undefined || settings.legs === undefined || settings.order === undefined) {
			untrack(() => emit({}));
		}
	});

	function setMinutes(e: Event) {
		const n = Math.trunc((e.currentTarget as HTMLInputElement).valueAsNumber);
		if (Number.isFinite(n)) emit({ match_minutes: Math.min(60, Math.max(1, n)) });
	}
</script>

<div class="ns">
	<label>
		{t.matchMinutes}
		<input type="number" inputmode="numeric" min="1" max="60" step="1" value={minutes} oninput={setMinutes} />
	</label>

	<div role="group" aria-labelledby="l-legs">
		<span class="label" id="l-legs">{t.legs}</span>
		<div class="seg">
			{#each LEGS as n (n)}
				<button type="button" aria-pressed={legs === n} onclick={() => emit({ legs: n })}>{n}</button>
			{/each}
		</div>
		<p class="hint">{t.legsOptions[legs as 1 | 2 | 3] ?? ''}</p>
	</div>

	<div role="group" aria-labelledby="l-order">
		<span class="label" id="l-order">{t.order}</span>
		<div class="seg">
			{#each ORDERS as o (o)}
				<button type="button" aria-pressed={order === o} onclick={() => emit({ order: o })}>{t.orderOptions[o]}</button>
			{/each}
		</div>
	</div>
</div>

<style>
	.ns {
		display: grid;
		gap: 16px;
	}
	.label {
		display: block;
		font-size: 13px;
		color: var(--muted);
		margin-bottom: 6px;
	}
	.seg {
		display: flex;
		background: var(--panel);
		border: 1px solid var(--line);
		border-radius: 12px;
		padding: 3px;
	}
	.seg button {
		flex: 1;
		min-width: 0;
		min-height: 42px;
		font-size: 14px;
		border: 0;
		background: transparent;
		color: var(--ink);
		padding: 8px 4px;
		border-radius: 9px;
		overflow-wrap: anywhere;
	}
	.seg button[aria-pressed='true'] {
		background: var(--ink);
		color: var(--glass);
		font-weight: 650;
	}
	.hint {
		margin: 6px 0 0;
	}
</style>
