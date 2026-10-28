<script lang="ts">
	// One week of the calendar for tablet and PC: seven days across, each with what starts that day.
	// The days are the device's own, Monday first. Items link to where they open.
	import { itemHref, itemNote, itemTime, itemTitle, byDay } from '#lib/calendar.ts';
	import { addDays, browserTimeZone, dayLabel, localMidnight } from '#lib/dates.ts';
	import { strings } from '#lib/strings.ts';
	import type { CalendarItem } from '#lib/types.ts';

	let {
		items,
		first,
		today
	}: {
		/** The items to place; those outside the week are left out. */
		items: CalendarItem[];
		/** The Monday the week starts on, "2026-11-02". */
		first: string;
		/** The device's today, "2026-11-04": its column is marked. */
		today: string;
	} = $props();

	const zone = browserTimeZone();
	const days = $derived(
		Array.from({ length: 7 }, (_, i) => {
			const date = addDays(first, i);
			return { date, label: dayLabel(localMidnight(date, zone) + 43200, zone) };
		})
	);
	const grouped = $derived(new Map(byDay(items, zone).map((d) => [d.date, d.items])));
</script>

<div class="week" role="group" aria-label={strings.calendar.week}>
	{#each days as day (day.date)}
		<section class:today={day.date === today} aria-label={day.label}>
			<h3>{day.label}</h3>
			{#each grouped.get(day.date) ?? [] as item, i (i)}
				<a href={itemHref(item)} class="item {item.kind}">
					<span class="time">{itemTime(item, zone)}</span>
					<span class="title">{itemTitle(item)}</span>
					<span class="note">{itemNote(item)}</span>
				</a>
			{:else}
				<p class="none" aria-hidden="true">·</p>
			{/each}
		</section>
	{/each}
</div>

<style>
	.week {
		display: grid;
		grid-template-columns: repeat(7, minmax(0, 1fr));
		gap: 6px;
	}
	section {
		display: grid;
		align-content: start;
		gap: 6px;
		min-width: 0;
		padding: 8px 6px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		min-height: 9rem;
	}
	section.today {
		border-color: var(--a);
		box-shadow: 0 0 0 1px var(--a);
	}
	h3 {
		margin: 0;
		font-size: 13px;
		font-weight: 750;
		color: var(--muted);
	}
	.item {
		display: grid;
		gap: 1px;
		min-height: var(--tap);
		padding: 6px;
		border-radius: 8px;
		background: var(--a-soft);
		color: var(--ink);
		text-decoration: none;
		min-width: 0;
	}
	.item.event {
		background: var(--b-soft);
	}
	.item.match {
		background: var(--glass);
		border: 1px solid var(--line);
	}
	.time {
		font-size: 12px;
		font-weight: 700;
	}
	.title {
		font-size: 13px;
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	.note {
		font-size: 11px;
		color: var(--muted);
	}
	.none {
		margin: 0;
		color: var(--line);
		text-align: center;
	}
</style>
