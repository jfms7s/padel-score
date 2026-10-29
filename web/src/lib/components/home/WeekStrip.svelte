<script lang="ts">
	// Seven days, Monday first, each with a pip per item that starts on it. Tapping a day asks the
	// page to scroll the agenda to it; ‹ › move the week.
	import { shiftWeek, weekStart } from '#lib/calendar.ts';
	import { addDays, dayLabel, localMidnight } from '#lib/dates.ts';
	import { weekPips } from '#lib/home.ts';
	import { strings } from '#lib/strings.ts';
	import type { CalendarItem } from '#lib/types.ts';

	let {
		items,
		first,
		today,
		timeZone,
		onweek,
		onday
	}: {
		items: CalendarItem[];
		first: string;
		today: string;
		timeZone: string;
		onweek: (first: string) => void;
		onday: (date: string) => void;
	} = $props();

	const t = strings.calendar;
	const label = (date: string) => dayLabel(localMidnight(date, timeZone) + 43200, timeZone);
	const days = $derived(
		weekPips(items, first, timeZone).map((d) => {
			const [weekday = '', num = ''] = label(d.date).split(' ');
			return { ...d, weekday, num, pips: d.kinds.slice(0, 3) };
		})
	);
</script>

<section aria-labelledby="week">
	<h2 id="week">{strings.home.week}</h2>
	<div class="nav">
		<button
			type="button"
			class="arrow"
			aria-label={t.prevWeek}
			disabled={first <= weekStart(today)}
			onclick={() => onweek(shiftWeek(first, -1))}>‹</button
		>
		<strong>{t.weekOf(label(first), label(addDays(first, 6)))}</strong>
		<button type="button" class="arrow" aria-label={t.nextWeek} onclick={() => onweek(shiftWeek(first, 1))}>›</button>
	</div>
	<div class="days">
		{#each days as d (d.date)}
			<button
				type="button"
				class="day"
				class:today={d.date === today}
				aria-label={label(d.date)}
				onclick={() => onday(d.date)}
			>
				<span class="wd">{d.weekday}</span>
				<b>{d.num}</b>
				<span class="pips" aria-hidden="true">
					{#each d.pips as kind, i (i)}<i class={kind}></i>{/each}
				</span>
			</button>
		{/each}
	</div>
</section>

<style>
	h2 {
		margin: 24px 0 8px;
		font-size: 18px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	.nav {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		margin: 0 0 8px;
	}
	.arrow {
		width: var(--tap);
		min-height: var(--tap);
		padding: 0;
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--panel);
		color: var(--ink);
		font-size: 22px;
	}
	.arrow:disabled {
		opacity: 0.5;
	}
	.days {
		display: grid;
		grid-template-columns: repeat(7, minmax(0, 1fr));
		gap: 4px;
	}
	.day {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 2px;
		min-height: var(--tap);
		padding: 6px 0;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		font-size: 12px;
	}
	.day b {
		font-size: 16px;
	}
	.day.today {
		border-color: var(--ink);
		background: var(--ink);
		color: var(--glass);
	}
	.pips {
		display: flex;
		gap: 2px;
		height: 6px;
	}
	.pips i {
		width: 6px;
		height: 6px;
		border-radius: 50%;
	}
	.pips i.game {
		background: var(--a);
	}
	.pips i.event {
		background: var(--ev);
	}
	.pips i.match {
		background: var(--b);
	}
</style>
