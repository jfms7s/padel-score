<!--
	Owned by phase 6 (the events foundation). The part of the event screen that belongs to its
	kind (the bracket, the standings), for the coordinator or a spectator: it hands the event to
	the kind's own component. Phases 7 and 8 fill those in. `parts` (the coordinator desk) asks a
	non-stop for only its clock or only its tables; a tournament has no clock to show.
-->
<script lang="ts">
	import type { EventDetail, EventView } from '#lib/types.ts';
	import NonStopPanel from './NonStopPanel.svelte';
	import TournamentPanel from './TournamentPanel.svelte';

	let {
		event,
		audience,
		parts = 'all'
	}: { event: EventDetail | EventView; audience: 'coordinator' | 'spectator'; parts?: 'all' | 'clock' | 'tables' } = $props();
</script>

{#if event.event.kind === 'tournament'}
	{#if parts !== 'clock'}<TournamentPanel {event} {audience} />{/if}
{:else}
	<NonStopPanel {event} {audience} {parts} />
{/if}
