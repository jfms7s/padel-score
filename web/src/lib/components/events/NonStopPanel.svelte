<!--
	Owned by phase 8 (the non-stop). The non-stop's part of the event screen, for the coordinator or a
	spectator: the clock (buttons for the coordinator only), the standings and the schedule. The data
	arrives in `kind_data` on the stream the page already holds. `parts` shows only the clock or only
	the tables (side by side on a desktop), for a page that places them apart; `clockVariant` is the
	clock's layout.
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

	let {
		event,
		audience,
		parts = 'all',
		clockVariant = 'full'
	}: {
		event: EventDetail | EventView;
		audience: 'coordinator' | 'spectator';
		parts?: 'all' | 'clock' | 'tables';
		clockVariant?: 'full' | 'strip';
	} = $props();

	const data = $derived(nonStopData(event.kind_data));

	onMount(() => void serverClock().sync());
</script>

{#if data}
	{#if parts !== 'tables'}
		<NonStopClock
			eventId={event.event.id}
			clock={data.clock}
			rounds={data.rounds}
			waves={data.waves}
			controls={audience === 'coordinator'}
			variant={clockVariant}
		/>
	{/if}
	{#if parts !== 'clock'}
		<div class="tables" class:split={parts === 'tables'}>
			<div>
				<h3>{t.standings}</h3>
				<Standings rows={data.standings} teams={event.teams} />
				<p class="hint">{t.standingsHint}</p>
			</div>
			<div>
				<h3>{t.schedule}</h3>
				<NonStopSchedule {data} {event} />
			</div>
		</div>
	{/if}
{/if}

<style>
	h3 {
		margin: 20px 0 8px;
		font-size: 16px;
		font-weight: 700;
	}
	.tables > div {
		min-width: 0;
	}
	@media (min-width: 1100px) {
		.split {
			display: grid;
			grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
			gap: 0 24px;
			align-items: start;
		}
	}
</style>
