<script lang="ts">
	// What is coming up, grouped by day. Each day group has an id so the week strip can scroll to it.
	import { byDay, itemHref, itemTime } from '#lib/calendar.ts';
	import { dayLabel, localMidnight } from '#lib/dates.ts';
	import { agendaNote, agendaTitle, isCancelled } from '#lib/home.ts';
	import { strings } from '#lib/strings.ts';
	import type { CalendarItem } from '#lib/types.ts';

	let { items, timeZone }: { items: CalendarItem[]; timeZone: string } = $props();

	const days = $derived(byDay(items, timeZone));
	const label = (date: string) => dayLabel(localMidnight(date, timeZone) + 43200, timeZone);
</script>

<section aria-labelledby="upcoming">
	<h2 id="upcoming">{strings.calendar.upcoming}</h2>
	{#if days.length === 0}
		<p class="hint">{strings.calendar.empty}</p>
	{:else}
		{#each days as day (day.date)}
			<div id="day-{day.date}" class="group">
				<h3>{label(day.date)}</h3>
				<ul>
					{#each day.items as item, i (`${day.date}:${i}`)}
						<li>
							<a class="row {item.kind}" class:cancelled={isCancelled(item)} href={itemHref(item)}>
								<span class="time">{itemTime(item, timeZone)}</span>
								<span class="what">
									<span class="title">
										{agendaTitle(item)}
										{#if item.kind !== 'game' && item.role === 'organiser'}
											<span class="chip">{strings.calendar.role.organiser}</span>
										{/if}
									</span>
									<span class="sub">{agendaNote(item)}</span>
								</span>
							</a>
						</li>
					{/each}
				</ul>
			</div>
		{/each}
	{/if}
</section>

<style>
	h2 {
		margin: 24px 0 8px;
		font-size: 18px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	h3 {
		margin: 16px 0 6px;
		font-size: 14px;
		color: var(--muted);
	}
	.hint {
		margin: 0 0 8px;
		color: var(--muted);
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 8px;
	}
	.row {
		display: flex;
		gap: 12px;
		align-items: center;
		min-height: var(--tap);
		padding: 8px 14px;
		border: 1px solid var(--line);
		border-left: 4px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		text-decoration: none;
	}
	.row.game {
		border-left-color: var(--a);
	}
	.row.event {
		border-left-color: var(--ev);
	}
	.row.match {
		border-left-color: var(--b);
	}
	.row.cancelled {
		opacity: 0.6;
	}
	.row.cancelled .title {
		text-decoration: line-through;
	}
	.time {
		flex: none;
		min-width: 4.5rem;
		font-weight: 700;
		font-size: 14px;
	}
	.what {
		display: grid;
		min-width: 0;
	}
	.title {
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	.chip {
		display: inline-block;
		padding: 0 8px;
		border-radius: 999px;
		background: var(--a-soft);
		font-size: 12px;
		font-weight: 650;
		line-height: 20px;
		vertical-align: middle;
	}
	.sub {
		font-size: 13px;
		color: var(--muted);
	}
</style>
