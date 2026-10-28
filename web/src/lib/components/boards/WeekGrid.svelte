<script lang="ts">
	// The board's week: free time painted in half hours. Tablet and PC have days down the side and
	// hours across; a phone turns it, days across and hours down. The view shows 07:00 and on but
	// the whole day scrolls. Drag to add a range, or tap one half hour and then another on the same
	// day. Tap a half hour to see who is free then; tap one of your ranges to trim it.
	import { browserTimeZone, dayLabel, timeText, timeRange, weekDays } from '#lib/dates.ts';
	import {
		SLOT_SECS,
		dayCells,
		freeAt,
		heatMap,
		mergedRanges,
		orientation,
		rangeFromCells,
		shade,
		type Range
	} from '#lib/grid.ts';
	import { strings } from '#lib/strings.ts';
	import type { BoardView } from '#lib/types.ts';

	let {
		view,
		me,
		readonly,
		onadd,
		onremove,
		ongame
	}: {
		view: BoardView;
		me: string | null;
		readonly: boolean;
		onadd: (start: number, end: number) => void;
		onremove: (start: number, end: number) => void;
		ongame: (gameId: string) => void;
	} = $props();

	const t = strings.boards;
	/** The view opens at 07:00: clock row 14. */
	const FIRST_ROW = 14;
	const LONG_PRESS_MS = 350;
	const MOVE_SLOP_PX = 10;

	// Days and times are the viewer's own; cells outside the board's week are switched off.
	const zone = browserTimeZone();
	let width = $state(1024);
	const vertical = $derived(orientation(width) === 'columns');

	interface GridCell {
		t: number;
		di: number;
		row: number;
		enabled: boolean;
		others: number;
		label: string;
	}

	const model = $derived.by(() => {
		const heat = heatMap(view.pieces, me, view.games);
		const days = weekDays(view.board.starts_at, view.board.ends_at, zone);
		const cells: GridCell[] = [];
		const byKey = new Map<string, GridCell>();
		const byTime = new Map<number, GridCell>();
		days.forEach((day, di) => {
			for (const c of dayCells(day, zone)) {
				const others = heat.get(c.t) ?? 0;
				const when = `${dayLabel(c.t, zone)} ${timeText(c.t, zone)}${c.row >= 48 ? ` ${t.secondTime}` : ''}`;
				const cell = {
					t: c.t,
					di,
					row: c.row,
					enabled: c.t >= view.board.starts_at && c.t < view.board.ends_at,
					others,
					label: t.cellLabel(when, others)
				};
				cells.push(cell);
				byKey.set(`${di}:${c.row}`, cell);
				byTime.set(c.t, cell);
			}
		});
		const rowCount = Math.max(48, ...cells.map((c) => c.row + 1));
		// A label for each row: the hour on the even clock rows, the repeated half hours of a
		// 25-hour day after them.
		const rowLabels = new Map<number, string>();
		for (const c of cells) {
			if (rowLabels.has(c.row)) continue;
			const clock = timeText(c.t, zone);
			if (c.row >= 48) rowLabels.set(c.row, clock);
			else if (c.row % 2 === 0) rowLabels.set(c.row, clock.slice(0, 2));
		}
		return { days, cells, byKey, byTime, rowCount, rowLabels };
	});

	const mine = $derived(me === null ? [] : mergedRanges(view.pieces, me));

	/** The runs of consecutive rows a range covers on each day it touches. */
	function segments(range: Range): { di: number; r0: number; r1: number }[] {
		const out: { di: number; r0: number; r1: number }[] = [];
		const inRange = model.cells.filter((c) => c.t >= range.start && c.t < range.end);
		for (const di of new Set(inRange.map((c) => c.di))) {
			const rows = inRange
				.filter((c) => c.di === di)
				.map((c) => c.row)
				.sort((a, b) => a - b);
			let r0 = rows[0] ?? 0;
			let prev = r0;
			for (const r of rows.slice(1)) {
				if (r !== prev + 1) {
					out.push({ di, r0, r1: prev });
					r0 = r;
				}
				prev = r;
			}
			out.push({ di, r0, r1: prev });
		}
		return out;
	}

	const mineSegments = $derived(mine.flatMap((range) => segments(range).map((s, i) => ({ range, ...s, first: i === 0 }))));
	const gameSegments = $derived(view.games.flatMap((game) => segments(game).map((s, i) => ({ game, ...s, first: i === 0 }))));

	/** Where something sits in the CSS grid: a day, and a run of rows on the other axis. */
	function place(di: number, r0: number, r1 = r0): string {
		const run = `${r0 + 2} / span ${r1 - r0 + 1}`;
		return vertical ? `grid-column: ${di + 2}; grid-row: ${run};` : `grid-row: ${di + 2}; grid-column: ${run};`;
	}

	const gridStyle = $derived(
		vertical
			? `grid-template-columns: 34px repeat(${model.days.length}, minmax(0, 1fr)); grid-template-rows: 30px repeat(${model.rowCount}, 28px);`
			: `grid-template-columns: 52px repeat(${model.rowCount}, 34px); grid-template-rows: 22px repeat(${model.days.length}, 44px);`
	);

	const when = (secs: number) =>
		`${dayLabel(secs, zone)} ${timeText(secs, zone)}${(model.byTime.get(secs)?.row ?? 0) >= 48 ? ` ${t.secondTime}` : ''}`;
	const rangeText = (r: Range) => `${dayLabel(r.start, zone)} ${timeRange(r.start, r.end, zone)}`;

	// ---- scrolling to 07:00 -------------------------------------------------------------

	let scroller: HTMLElement | undefined = $state();
	let scrolledFor: boolean | null = null;

	$effect(() => {
		if (!scroller || scrolledFor === vertical) return;
		scrolledFor = vertical;
		const el = scroller;
		const wide = !vertical;
		// After the grid has been laid out in its new orientation.
		requestAnimationFrame(() => {
			if (wide) el.scrollLeft = FIRST_ROW * 34;
			else el.scrollTop = FIRST_ROW * 28;
		});
	});

	// ---- selecting, tapping and dragging ------------------------------------------------

	/** The half hour tapped first, waiting for a second one (or just showing who is free). */
	let anchor = $state<number | null>(null);
	let drag = $state<{ from: number; to: number } | null>(null);
	let focusT = $state<number | null>(null);

	interface Press {
		cell: GridCell;
		pointerId: number;
		x: number;
		y: number;
		touch: boolean;
		dragging: boolean;
		timer: ReturnType<typeof setTimeout> | null;
	}
	let press: Press | null = null;

	const defaultFocus = $derived(model.cells.find((c) => c.di === 0 && c.row === FIRST_ROW)?.t ?? model.cells[0]?.t ?? 0);
	const focusAt = $derived(focusT !== null && model.byTime.has(focusT) ? focusT : defaultFocus);

	const freeNow = $derived(anchor === null ? [] : freeAt(view, anchor));
	const anchorCell = $derived(anchor === null ? undefined : model.byTime.get(anchor));

	function cellOf(el: Element | null): GridCell | undefined {
		const raw = el?.closest<HTMLElement>('[data-t]')?.dataset.t;
		return raw === undefined ? undefined : model.byTime.get(Number(raw));
	}

	function cellAt(x: number, y: number): GridCell | undefined {
		return document
			.elementsFromPoint(x, y)
			.map((el) => (el instanceof HTMLElement && el.dataset.t !== undefined ? model.byTime.get(Number(el.dataset.t)) : undefined))
			.find((c) => c !== undefined);
	}

	function tap(cell: GridCell) {
		if (!cell.enabled) return;
		focusT = cell.t;
		if (anchor === cell.t) {
			anchor = null;
		} else if (anchor === null || readonly || model.byTime.get(anchor)?.di !== cell.di) {
			anchor = cell.t;
		} else {
			const range = rangeFromCells(anchor, cell.t);
			anchor = null;
			onadd(range.start, range.end);
		}
	}

	function clearPress() {
		if (press?.timer) clearTimeout(press.timer);
		press = null;
		drag = null;
	}

	function down(e: PointerEvent) {
		if (e.button !== 0 || !(e.target instanceof Element)) return;
		const cell = cellOf(e.target);
		if (!cell || !cell.enabled || e.target.closest('.mine, .game')) return;
		clearPress();
		const touch = e.pointerType === 'touch';
		press = { cell, pointerId: e.pointerId, x: e.clientX, y: e.clientY, touch, dragging: !touch, timer: null };
		if (touch) {
			// A finger scrolls; holding still for a moment starts a drag instead.
			press.timer = setTimeout(() => {
				if (press) {
					press.dragging = true;
					drag = { from: press.cell.t, to: press.cell.t };
				}
			}, LONG_PRESS_MS);
		} else {
			(e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
			drag = { from: cell.t, to: cell.t };
		}
	}

	function move(e: PointerEvent) {
		if (!press || e.pointerId !== press.pointerId) return;
		if (!press.dragging) {
			if (Math.hypot(e.clientX - press.x, e.clientY - press.y) > MOVE_SLOP_PX) clearPress();
			return;
		}
		const over = cellAt(e.clientX, e.clientY);
		if (over?.enabled && over.di === press.cell.di && drag) drag = { from: drag.from, to: over.t };
	}

	function up(e: PointerEvent) {
		if (!press || e.pointerId !== press.pointerId) return;
		const { cell } = press;
		const moved = drag !== null && drag.from !== drag.to;
		const range = drag && moved ? rangeFromCells(drag.from, drag.to) : null;
		clearPress();
		if (range && !readonly && cell.enabled) {
			anchor = null;
			onadd(range.start, range.end);
		} else if (!moved) {
			tap(cell);
		}
	}

	function click(e: MouseEvent) {
		// A keyboard press on a focused half hour; pointer taps are handled on release.
		if (e.detail !== 0) return;
		const cell = cellOf(e.target instanceof Element ? e.target : null);
		if (cell) tap(cell);
	}

	function keydown(e: KeyboardEvent) {
		const cell = cellOf(e.target instanceof Element ? e.target : null);
		if (!cell) return;
		const along = vertical ? ['ArrowUp', 'ArrowDown'] : ['ArrowLeft', 'ArrowRight'];
		const across = vertical ? ['ArrowLeft', 'ArrowRight'] : ['ArrowUp', 'ArrowDown'];
		let di = cell.di;
		let row = cell.row;
		if (e.key === along[0]) row -= 1;
		else if (e.key === along[1]) row += 1;
		else if (e.key === across[0]) di -= 1;
		else if (e.key === across[1]) di += 1;
		else return;
		e.preventDefault();
		const next = model.byKey.get(`${di}:${row}`);
		if (!next) return;
		focusT = next.t;
		queueMicrotask(() => scroller?.querySelector<HTMLElement>(`[data-t="${next.t}"]`)?.focus());
	}

	// A held finger that has started a drag must not also scroll the grid.
	$effect(() => {
		const el = scroller;
		if (!el) return;
		const stop = (e: TouchEvent) => {
			if (press?.dragging) e.preventDefault();
		};
		el.addEventListener('touchmove', stop, { passive: false });
		return () => el.removeEventListener('touchmove', stop);
	});

	const inDrag = (cell: GridCell) => {
		if (!drag) return false;
		const day = model.byTime.get(drag.from)?.di;
		return cell.di === day && cell.t >= Math.min(drag.from, drag.to) && cell.t <= Math.max(drag.from, drag.to);
	};

	// ---- the trim sheet ----------------------------------------------------------------

	let dialog: HTMLDialogElement | undefined = $state();
	let trim = $state<Range | null>(null);
	let from = $state(0);
	let to = $state(0);

	const slotsOf = (r: Range) => Array.from({ length: (r.end - r.start) / SLOT_SECS }, (_, i) => r.start + i * SLOT_SECS);
	const fromOptions = $derived(trim ? slotsOf(trim) : []);
	const toOptions = $derived(trim ? slotsOf(trim).map((s) => s + SLOT_SECS).filter((e) => e > from) : []);

	function openTrim(range: Range) {
		trim = range;
		from = range.start;
		to = range.end;
	}

	$effect(() => {
		if (!dialog) return;
		if (trim && !dialog.open) dialog.showModal();
		if (!trim && dialog.open) dialog.close();
	});

	function pickFrom(value: number) {
		from = value;
		if (to <= from) to = from + SLOT_SECS;
	}

	function saveTrim() {
		if (!trim) return;
		const range = trim;
		trim = null;
		// Cut away what is left out at either end.
		if (from > range.start) onremove(range.start, from);
		if (to < range.end) onremove(to, range.end);
	}

	function removeTrim() {
		if (!trim) return;
		const range = trim;
		trim = null;
		onremove(range.start, range.end);
	}
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && (anchor = null)} />

<section class="week" bind:clientWidth={width} aria-label={t.grid}>
	{#if !readonly}<p class="hint">{t.gridHint}</p>{/if}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="scroller"
		class:cols={vertical}
		bind:this={scroller}
		onpointerdown={down}
		onpointermove={move}
		onpointerup={up}
		onpointercancel={clearPress}
		onclick={click}
		onkeydown={keydown}
	>
		<div class="grid" class:cols={vertical} style={gridStyle} role="group" aria-label={t.grid}>
			<span class="corner" style="grid-row: 1; grid-column: 1;"></span>

			{#each model.days as day, di (day.date)}
				<span class="daylab" style={vertical ? `grid-row: 1; grid-column: ${di + 2};` : `grid-column: 1; grid-row: ${di + 2};`}>
					{dayLabel(day.start, zone)}
				</span>
			{/each}
			{#each [...model.rowLabels] as [row, label] (row)}
				<span
					class="hourlab"
					class:again={row >= 48}
					style={vertical ? `grid-column: 1; grid-row: ${row + 2};` : `grid-row: 1; grid-column: ${row + 2} / span 2;`}>{label}</span
				>
			{/each}

			{#each model.cells as cell (cell.t)}
				<button
					type="button"
					class="cell heat-{shade(cell.others)}"
					class:off={!cell.enabled}
					class:pending={inDrag(cell)}
					class:anchor={anchor === cell.t}
					class:hour={cell.row % 2 === 0}
					class:again={cell.row >= 48}
					style={place(cell.di, cell.row)}
					data-t={cell.t}
					tabindex={cell.t === focusAt ? 0 : -1}
					aria-label={cell.label}
					aria-pressed={anchor === cell.t}
				></button>
			{/each}

			{#each mineSegments as seg (`${seg.range.start}:${seg.di}:${seg.r0}`)}
				<div class="mine" style={place(seg.di, seg.r0, seg.r1)}>
					{#if readonly}
						<span class="body"><span class="text">{timeRange(seg.range.start, seg.range.end, zone)}</span></span>
					{:else}
						<button type="button" class="body" aria-label={t.editRange(rangeText(seg.range))} onclick={() => openTrim(seg.range)}>
							<span class="text">{timeRange(seg.range.start, seg.range.end, zone)}</span>
						</button>
						{#if seg.first}
							<button type="button" class="x" aria-label={t.removeRange(rangeText(seg.range))} onclick={() => onremove(seg.range.start, seg.range.end)}>
								<span aria-hidden="true">✕</span>
							</button>
						{/if}
					{/if}
				</div>
			{/each}

			{#each gameSegments as seg (`${seg.game.id}:${seg.di}:${seg.r0}`)}
				<!-- An outline that lets taps through to the range under it; the button opens the game. -->
				<div class="game" style={place(seg.di, seg.r0, seg.r1)}>
					{#if seg.first}
						<button
							type="button"
							class="gopen"
							aria-label={t.gameLabel(rangeText(seg.game), t.gameState[seg.game.state])}
							onclick={() => ongame(seg.game.id)}
						>
							<span aria-hidden="true">●</span>
						</button>
					{/if}
				</div>
			{/each}
		</div>
	</div>

	<ul class="legend" aria-hidden="true">
		<li><span class="sw mine-sw"></span>{t.yourTime}</li>
		<li><span class="sw some"></span>{t.legendOthers1}</li>
		<li><span class="sw game-heat"></span>{t.legendOthers3}</li>
		<li><span class="sw game-sw"></span>{t.legendGame}</li>
	</ul>

	<div class="free" aria-live="polite">
		{#if anchorCell}
			<h3>{t.freeAt(when(anchorCell.t))}</h3>
			{#if freeNow.length === 0}
				<p class="hint">{t.nobodyFree}</p>
			{:else}
				<ol>
					{#each freeNow as m (m.user_id)}
						<li>{m.name}{#if m.user_id === me}<span class="you"> ({t.you})</span>{/if}</li>
					{/each}
				</ol>
			{/if}
			{#if !readonly}<p class="hint">{t.tapAnother}</p>{/if}
		{/if}
	</div>
</section>

<dialog bind:this={dialog} onclose={() => (trim = null)} aria-labelledby="trim-title">
	<div class="sheet">
		<h2 id="trim-title">{t.trimTitle}</h2>
		{#if trim}
			<div class="fields">
				<label>
					{t.trimFrom}
					<select value={from} onchange={(e) => pickFrom(Number(e.currentTarget.value))}>
						{#each fromOptions as s (s)}<option value={s}>{when(s)}</option>{/each}
					</select>
				</label>
				<label>
					{t.trimTo}
					<select bind:value={to}>
						{#each toOptions as s (s)}<option value={s}>{when(s)}</option>{/each}
					</select>
				</label>
			</div>
		{/if}
		<div class="buttons">
			<button type="button" class="primary" onclick={saveTrim}>{strings.boards.save}</button>
			<button type="button" class="danger" onclick={removeTrim}>{t.trimRemove}</button>
			<button type="button" class="secondary" onclick={() => (trim = null)}>{t.trimClose}</button>
		</div>
	</div>
</dialog>

<style>
	.week {
		min-width: 0;
	}
	.hint {
		margin: 0 0 8px;
		font-size: 13px;
		color: var(--muted);
	}
	.scroller {
		overflow-x: auto;
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--panel);
		user-select: none;
		-webkit-user-select: none;
		overscroll-behavior: contain;
	}
	.scroller.cols {
		overflow-x: hidden;
		overflow-y: auto;
		max-height: min(70vh, 640px);
	}
	.grid {
		display: grid;
		width: max-content;
		min-width: 100%;
		position: relative;
	}
	.grid.cols {
		width: 100%;
	}
	.corner,
	.daylab,
	.hourlab {
		position: sticky;
		z-index: 5;
		background: var(--panel);
		font-size: 12px;
		color: var(--muted);
	}
	.corner {
		left: 0;
		top: 0;
		z-index: 6;
	}
	.daylab {
		display: flex;
		align-items: center;
		font-weight: 700;
		color: var(--ink);
		white-space: nowrap;
	}
	.grid:not(.cols) .daylab {
		left: 0;
		padding-left: 8px;
		font-size: 13px;
	}
	.grid.cols .daylab {
		top: 0;
		justify-content: center;
		font-size: 11px;
		border-bottom: 1px solid var(--line);
	}
	.hourlab {
		line-height: 1;
	}
	.grid:not(.cols) .hourlab {
		top: 0;
		padding-top: 4px;
		padding-left: 3px;
	}
	.grid.cols .hourlab {
		left: 0;
		padding: 3px 0 0 4px;
	}
	.hourlab.again {
		color: var(--warn);
	}

	.cell {
		min-height: 0;
		padding: 0;
		border: 0;
		border-top: 1px solid var(--line);
		border-left: 1px solid var(--line);
		border-radius: 0;
		background: transparent;
		touch-action: pan-x pan-y;
	}
	.cell.hour {
		border-left-color: color-mix(in srgb, var(--ink) 25%, var(--line));
	}
	.grid.cols .cell {
		border-left: 1px solid var(--line);
	}
	.grid.cols .cell.hour {
		border-left-color: var(--line);
		border-top-color: color-mix(in srgb, var(--ink) 25%, var(--line));
	}
	.cell.again {
		border-style: dashed;
	}
	.cell.heat-some {
		background: color-mix(in srgb, var(--b) 22%, transparent);
	}
	.cell.heat-game {
		background: color-mix(in srgb, var(--b) 55%, transparent);
	}
	.cell.pending {
		background: color-mix(in srgb, var(--a) 35%, transparent);
	}
	.cell.anchor {
		background: color-mix(in srgb, var(--a) 45%, transparent);
		outline: 2px solid var(--a);
		outline-offset: -2px;
	}
	.cell.off {
		opacity: 0.35;
	}
	.cell:focus-visible {
		outline: 3px solid var(--a);
		outline-offset: -3px;
		position: relative;
		z-index: 4;
	}

	.mine {
		position: relative;
		z-index: 2;
		pointer-events: none;
		margin: 3px 2px;
	}
	.grid.cols .mine {
		margin: 0 3px;
	}
	.mine .body {
		position: absolute;
		inset: 0;
		display: flex;
		align-items: center;
		min-height: 0;
		padding: 0 6px;
		border: 0;
		border-top: 2px solid var(--a);
		border-radius: 4px;
		background: var(--a-soft);
		color: var(--ink);
		font-size: 11px;
		text-align: left;
		pointer-events: auto;
		overflow: hidden;
	}
	.grid.cols .mine .body {
		border-top: 0;
		border-left: 3px solid var(--a);
		padding: 0 2px;
		font-size: 10px;
	}
	.grid.cols .mine .text {
		display: none;
	}
	.mine .text {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		padding-right: 22px;
	}
	.x {
		position: absolute;
		top: 50%;
		right: 0;
		translate: 0 -50%;
		z-index: 4;
		display: grid;
		place-items: center;
		width: 28px;
		min-height: 28px;
		height: 28px;
		padding: 0;
		border: 0;
		border-radius: 50%;
		background: transparent;
		color: var(--ink);
		font-size: 12px;
		pointer-events: auto;
	}
	.x::before {
		content: '';
		position: absolute;
		inset: -10px;
	}
	.grid.cols .x {
		top: 0;
		right: 0;
		translate: none;
		width: 22px;
		min-height: 22px;
		height: 22px;
	}

	.game {
		position: relative;
		z-index: 3;
		pointer-events: none;
		min-height: 0;
		margin: 1px;
		padding: 0;
		border: 3px solid var(--ball);
		border-radius: 5px;
		background: transparent;
		box-shadow: 0 0 0 1px var(--ink);
	}

	.gopen {
		position: absolute;
		left: 0;
		bottom: 0;
		z-index: 5;
		display: grid;
		place-items: center;
		width: 22px;
		min-height: 22px;
		height: 22px;
		padding: 0;
		border: 0;
		border-radius: 50%;
		background: var(--ball);
		color: var(--on-ball);
		font-size: 10px;
		pointer-events: auto;
	}
	.gopen::before {
		content: '';
		position: absolute;
		inset: -10px;
	}

	.legend {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 14px;
		margin: 10px 0 0;
		padding: 0;
		list-style: none;
		font-size: 12px;
		color: var(--muted);
	}
	.sw {
		display: inline-block;
		width: 12px;
		height: 12px;
		margin-right: 5px;
		border-radius: 3px;
		vertical-align: -2px;
	}
	.mine-sw {
		background: var(--a-soft);
		border-top: 2px solid var(--a);
	}
	.sw.some {
		background: color-mix(in srgb, var(--b) 22%, transparent);
		border: 1px solid var(--line);
	}
	.game-heat {
		background: color-mix(in srgb, var(--b) 55%, transparent);
	}
	.game-sw {
		border: 2px solid var(--ball);
		box-shadow: 0 0 0 1px var(--ink);
	}

	.free h3 {
		margin: 14px 0 4px;
		font-size: 15px;
	}
	.free ol {
		margin: 0 0 6px;
		padding-left: 22px;
	}
	.free .you {
		color: var(--muted);
	}

	dialog {
		width: min(100vw - 32px, 26rem);
		padding: 0;
		border: 1px solid var(--line);
		border-radius: 18px;
		background: var(--panel);
		color: var(--ink);
	}
	dialog::backdrop {
		background: rgba(5, 11, 18, 0.55);
	}
	.sheet {
		padding: 20px 18px;
	}
	h2 {
		margin: 0 0 14px;
		font-size: 22px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	.fields {
		display: grid;
		gap: 12px;
		margin-bottom: 16px;
	}
	label {
		display: flex;
		flex-direction: column;
		gap: 6px;
		font-size: 13px;
		color: var(--muted);
	}
	select {
		min-height: var(--tap);
		padding: 0 10px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		font-size: 16px;
	}
	.buttons {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	.buttons button {
		min-height: var(--tap);
		padding: 0 18px;
		border-radius: 14px;
		font-size: 16px;
		font-weight: 650;
	}
	.primary {
		border: 0;
		background: var(--ink);
		color: var(--glass);
	}
	.secondary {
		border: 1px solid var(--line);
		background: var(--panel);
		color: var(--ink);
	}
	.danger {
		border: 1px solid var(--warn);
		background: transparent;
		color: var(--warn);
	}
</style>
