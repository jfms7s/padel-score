<!--
	Owned by phase 8 (the non-stop). The non-stop's part of the event screen, for the coordinator or a
	spectator: the clock (buttons for the coordinator only), the standings and the schedule. The data
	arrives in `kind_data` on the stream the page already holds.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { nonStopData, serverClock } from '#lib/nonstop.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventDetail, EventView } from '#lib/types.ts';
	import NonStopClock from './NonStopClock.svelte';
	import NonStopSchedule from './NonStopSchedule.svelte';
	import Standings from './Standings.svelte';

	const t = strings.nonStop;

	let { event, audience }: { event: EventDetail | EventView; audience: 'coordinator' | 'spectator' } = $props();

	const data = $derived(nonStopData(event.kind_data));

	onMount(() => void serverClock().sync());
</script>

{#if data}
	<NonStopClock
		eventId={event.event.id}
		clock={data.clock}
		rounds={data.rounds}
		waves={data.waves}
		controls={audience === 'coordinator'}
	/>
	<h3>{t.standings}</h3>
	<Standings rows={data.standings} teams={event.teams} />
	<p class="hint">{t.standingsHint}</p>
	<h3>{t.schedule}</h3>
	<NonStopSchedule {data} {event} />
{/if}

<style>
	h3 {
		margin: 20px 0 8px;
		font-size: 16px;
		font-weight: 700;
	}
</style>
