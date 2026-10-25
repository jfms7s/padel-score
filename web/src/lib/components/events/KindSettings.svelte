<!--
	Owned by phase 6 (the events foundation). The settings that belong to an event's kind, on the
	new-event form: it hands them to the kind's own component. Phases 7 and 8 fill those in;
	this file stays as it is.
-->
<script lang="ts">
	import type { EventKind } from '#lib/types.ts';
	import NonStopSettings from './NonStopSettings.svelte';
	import TournamentSettings from './TournamentSettings.svelte';

	let {
		kind,
		settings,
		onchange
	}: {
		kind: EventKind;
		/** The event's `settings` object, as the server keeps it. */
		settings: Record<string, unknown>;
		onchange: (settings: Record<string, unknown>) => void;
	} = $props();
</script>

{#if kind === 'tournament'}
	<TournamentSettings {settings} {onchange} />
{:else}
	<NonStopSettings {settings} {onchange} />
{/if}
