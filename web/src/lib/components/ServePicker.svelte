<!--
	Who serves first in a match made elsewhere (an event match on the court phone): the four
	players, then the other pair's first server, as on New match. `onstart` saves the choice.
-->
<!--
	Who serves first in a match made elsewhere (an event match on the court phone): the same
	three choices as New match, picked from the match's current start. `onstart` saves them.
-->
<script lang="ts">
	import ServeOrderFields from '#lib/components/ServeOrderFields.svelte';
	import { defaultServeOrder, fromStart, toStart } from '#lib/serve.ts';
	import { strings } from '#lib/strings.ts';
	import type { Start, Teams } from '#lib/types.ts';

	let {
		teams,
		start,
		busy = false,
		failure = null,
		onstart
	}: {
		teams: Teams;
		/** The match's current choice, shown picked at first. */
		start: Start;
		busy?: boolean;
		failure?: string | null;
		onstart: (start: Start) => void;
	} = $props();

	const t = strings.court;

	// Seeded once from the match's choice; the phone's own taps change it after that.
	// svelte-ignore state_referenced_locally
	let order = $state(fromStart(start) ?? defaultServeOrder());

	function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		onstart(toStart(order));
	}
</script>

<form class="serve" onsubmit={submit}>
	<h2>{t.serveTitle}</h2>
	<ServeOrderFields bind:order {teams} />

	{#if failure}<p class="error" role="alert">{failure}</p>{/if}
	<button class="go" type="submit" disabled={busy}>{busy ? t.starting : t.startScoring}</button>
</form>

<style>
	.serve {
		display: flex;
		flex-direction: column;
		gap: 16px;
	}
	h2 {
		margin: 0;
		font-size: 22px;
		font-weight: 750;
	}
	.error {
		margin: 0;
		padding: 10px;
		border: 1px solid var(--warn);
		border-radius: 12px;
		color: var(--warn);
	}
	.go {
		min-height: 64px;
		border: 0;
		border-radius: 16px;
		background: var(--ink);
		color: var(--glass);
		font-size: 20px;
		font-weight: 750;
	}
	.go:disabled {
		opacity: 0.6;
	}
</style>
