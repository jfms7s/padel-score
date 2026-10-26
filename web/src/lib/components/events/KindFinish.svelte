<!--
	Owned by phase 6 (the events foundation). The body of the coordinator's Finish screen. When
	the kind leaves the places to the coordinator (`places_computed` is false) it shows the
	hand-ordering editor; when the kind computes them, it hands over to the kind's own component,
	which finishes with no places. Phases 7 and 8 fill in their own component and flip their
	`computes_places`; this file stays as it is.
-->
<script lang="ts">
	import type { EventDetail, Place } from '#lib/types.ts';
	import HandPlaces from './HandPlaces.svelte';
	import NonStopFinish from './NonStopFinish.svelte';
	import TournamentFinish from './TournamentFinish.svelte';

	let {
		event,
		busy,
		onfinish,
		oncancel
	}: {
		event: EventDetail;
		busy: boolean;
		/** Finish the event: with the places, or with none when the kind computes them. */
		onfinish: (places?: Place[]) => void;
		oncancel: () => void;
	} = $props();
</script>

{#if !event.places_computed}
	<HandPlaces {event} {busy} {onfinish} {oncancel} />
{:else if event.event.kind === 'tournament'}
	<TournamentFinish {event} {busy} {onfinish} {oncancel} />
{:else}
	<NonStopFinish {event} {busy} {onfinish} {oncancel} />
{/if}
