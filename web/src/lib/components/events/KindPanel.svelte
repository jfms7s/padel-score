<!--
	Owned by phase 6 (the events foundation). The part of the event screen that belongs to its
	kind (the bracket, the standings), for the coordinator or a spectator: it hands the event to
	the kind's own component. Phases 7 and 8 fill those in; this file stays as it is.
-->
<script lang="ts">
	import type { EventDetail, EventView } from '#lib/types.ts';
	import NonStopPanel from './NonStopPanel.svelte';
	import TournamentPanel from './TournamentPanel.svelte';

	let { event, audience }: { event: EventDetail | EventView; audience: 'coordinator' | 'spectator' } = $props();
</script>

{#if event.event.kind === 'tournament'}
	<TournamentPanel {event} {audience} />
{:else}
	<NonStopPanel {event} {audience} />
{/if}
