<script lang="ts">
	// The week as a grid for PCs: an hour column and seven days. Each item is a block placed by its
	// start and length; blocks that overlap share the day's width. Open-to-join blocks are dashed links.
	import { itemHref, shiftWeek, weekStart } from '#lib/calendar.ts';
	import { addDays, dayLabel, localDate, localMidnight, timeText } from '#lib/dates.ts';
	import { agendaTitle, isCancelled, itemEnd, itemKey, laneLayout, openEnd, shownHours } from '#lib/home.ts';
	import { strings } from '#lib/strings.ts';
	import type { CalendarItem, OpenItem } from '#lib/types.ts';

	let {
		items,
		open,
		first,
		today,
		timeZone,
		selected,
		onselect,
		onweek
	}: {
		items: CalendarItem[];
		/** Open to join: dashed links that can't be selected. */
		open: OpenItem[];
		/** The Monday the week starts on. */
		first: string;
		today: string;
		timeZone: string;
		/** The key of the selected item, if any. */
		selected: string | null;
		onselect: (key: string) => void;
		onweek: (first: string) => void;
	} = $props();

	const ROW = 34;
	const FOLD = 14;
	const t = strings.calendar;

	type Block = {
		key: string;
		kind: CalendarItem['kind'];
		isOpen: boolean;
		cancelled: boolean;
		href: string;
		title: string;
		start: number;
		end: number;
		time: string;
	};

	const label = (date: string) => dayLabel(localMidnight(date, timeZone) + 43200, timeZone);
	const hours = $derived(shownHours(items, first, timeZone, open));
	const days = $derived(Array.from({ length: 7 }, (_, i) => addDays(first, i)));

	// Hour rows, with a fold wherever the shown hours skip ahead.
	const rows = $derived(
		hours.map((hour, i) => ({ hour, fold: i > 0 && hour !== (hours[i - 1] ?? hour) + 1 }))
	);
	const hourText = (hour: number) => `${String(hour).padStart(2, '0')}:00`;

	const blocks = $derived.by(() => {
		const all: Block[] = [
			...items.map((item) => ({
				key: itemKey(item),
				kind: item.kind,
				isOpen: false,
				cancelled: isCancelled(item),
				href: itemHref(item),
				title: agendaTitle(item),
				start: item.kind === 'game' ? item.start : item.starts_at,
				end: itemEnd(item),
				time: ''
			})),
			...open.map((o) => ({
				key: `open:${o.join_token}`,
				kind: o.kind,
				isOpen: true,
				cancelled: false,
				href: itemHref(o),
				title: o.kind === 'event' ? o.name : strings.home.matchBy(o.owner_name),
				start: o.starts_at,
				end: openEnd(o),
				time: ''
			}))
		];
		const byDate = new Map<string, Block[]>();
		for (const b of all) {
			const date = localDate(b.start, timeZone);
			if (!days.includes(date)) continue;
			byDate.set(date, [...(byDate.get(date) ?? []), { ...b, time: timeText(b.start, timeZone) }]);
		}
		return byDate;
	});

	// Per day and start hour: the blocks, each with its box. Height is clipped to the run of
	// consecutive shown rows that follows, so a block never reaches into a fold or off the grid.
	const cells = $derived.by(() => {
		const out = new Map<string, { block: Block; top: number; height: number; left: number; width: number }[]>();
		for (const [date, list] of blocks) {
			const lanes = laneLayout(list);
			list.forEach((block, i) => {
				const lane = lanes[i] ?? { lane: 0, lanes: 1 };
				const hour = Number(timeText(block.start, timeZone).slice(0, 2));
				const minutes = Number(timeText(block.start, timeZone).slice(3, 5));
				let at = hours.indexOf(hour);
				if (at === -1) return;
				let run = 1;
				while (hours[at + run] === hour + run) run++;
				const top = (minutes / 60) * ROW;
				const height = Math.min(((block.end - block.start) / 3600) * ROW, run * ROW - top);
				const key = `${date}:${hour}`;
				out.set(key, [
					...(out.get(key) ?? []),
					{ block, top, height, left: (lane.lane / lane.lanes) * 100, width: 100 / lane.lanes }
				]);
			});
		}
		return out;
	});
</script>

<section aria-labelledby="agenda-week">
	<div class="head">
		<h2 id="agenda-week">{strings.home.week}</h2>
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
	</div>
	<div class="grid">
		<div class="corner"></div>
		{#each days as date (date)}
			<div class="dayhead" class:today={date === today}>{label(date)}</div>
		{/each}
		{#each rows as row (row.hour)}
			{#if row.fold}
				<div class="fold" style:height="{FOLD}px" aria-hidden="true">…</div>
				{#each days as date (date)}<div class="foldcell" style:height="{FOLD}px"></div>{/each}
			{/if}
			<div class="hour" style:height="{ROW}px">{hourText(row.hour)}</div>
			{#each days as date (date)}
				<div class="cell" style:height="{ROW}px">
					{#each cells.get(`${date}:${row.hour}`) ?? [] as c (c.block.key)}
						{#if c.block.isOpen}
							<a
								class="block open {c.block.kind}"
								href={c.block.href}
								style:top="{c.top}px"
								style:height="{c.height}px"
								style:left="{c.left}%"
								style:width="{c.width}%"
							>
								<b>{c.block.time}</b>
								<span>{c.block.title}</span>
							</a>
						{:else}
							<button
								type="button"
								class="block {c.block.kind}"
								class:selected={c.block.key === selected}
				class:cancelled={c.block.cancelled}
								aria-pressed={c.block.key === selected}
								style:top="{c.top}px"
								style:height="{c.height}px"
								style:left="{c.left}%"
								style:width="{c.width}%"
								onclick={() => onselect(c.block.key)}
							>
								<b>{c.block.time}</b>
								<span>{c.block.title}</span>
							</button>
						{/if}
					{/each}
				</div>
			{/each}
		{/each}
	</div>
</section>

<style>
	.head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		margin: 0 0 8px;
	}
	h2 {
		margin: 0;
		font-size: 18px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	.nav {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.arrow {
		width: 36px;
		min-height: 36px;
		padding: 0;
		border: 1px solid var(--line);
		border-radius: 10px;
		background: var(--panel);
		color: var(--ink);
		font-size: 20px;
	}
	.arrow:disabled {
		opacity: 0.5;
	}
	.grid {
		display: grid;
		grid-template-columns: 44px repeat(7, minmax(0, 1fr));
		border-top: 1px solid var(--line);
		border-left: 1px solid var(--line);
	}
	.grid > * {
		min-width: 0;
		border-right: 1px solid var(--line);
		border-bottom: 1px solid var(--line);
	}
	.dayhead {
		padding: 6px 4px;
		font-size: 12px;
		font-weight: 650;
		color: var(--muted);
		text-align: center;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.dayhead.today {
		color: var(--ink);
		font-weight: 800;
	}
	.hour,
	.fold {
		padding: 2px 4px;
		font-size: 11px;
		color: var(--muted);
		text-align: right;
	}
	.fold {
		line-height: 1;
		text-align: center;
	}
	.cell {
		position: relative;
	}
	.block {
		position: absolute;
		z-index: 1;
		display: flex;
		flex-direction: column;
		gap: 0;
		min-width: 0;
		padding: 2px 4px;
		overflow: hidden;
		border: 0;
		border-radius: 6px;
		color: var(--on-team);
		font: inherit;
		font-size: 11px;
		line-height: 1.2;
		text-align: left;
		text-decoration: none;
		cursor: pointer;
	}
	.block b {
		font-weight: 750;
	}
	.block span {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	/* The court tokens are --a, --b and --ev darkened to carry the white text. */
	.block.game {
		background: var(--a-court);
	}
	.block.event {
		background: var(--ev-court);
	}
	.block.match {
		background: var(--b-court);
	}
	.block.cancelled {
		opacity: 0.55;
	}
	.block.cancelled span {
		text-decoration: line-through;
	}
	.block.selected {
		outline: 2px solid var(--ball);
		outline-offset: 0;
	}
	.block:focus-visible {
		outline: 2px solid var(--ink);
		outline-offset: 1px;
	}
	.block.selected:focus-visible {
		outline-color: var(--ball);
		box-shadow: 0 0 0 4px var(--ink);
	}
	.block.open {
		background: transparent;
		border: 1.5px dashed var(--muted);
		color: var(--ink);
	}
</style>
