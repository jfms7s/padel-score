<!--
	Owned by phase 6's seam; phases 7 and 8 fill their own. The part of the court page that
	belongs to the event's kind (the non-stop's clock), above the match: it hands the court to
	the kind's own component. `viewer` is a spectator's court page: it shows, it makes no sound.
	This file stays as it is.
-->
<script lang="ts">
	import type { CourtState } from '#lib/types.ts';
	import NonStopCourtBanner from './NonStopCourtBanner.svelte';
	import TournamentCourtBanner from './TournamentCourtBanner.svelte';

	let { court, viewer = false }: { court: CourtState; viewer?: boolean } = $props();
</script>

{#if court.event.kind === 'tournament'}
	<TournamentCourtBanner />
{:else}
	<NonStopCourtBanner {court} {viewer} />
{/if}
