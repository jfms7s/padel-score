<script lang="ts">
	// The fence screen upright: a broadcast-style scoreboard. Each team gets a full-width block,
	// the set scores on the left and the points as big as the screen allows. The tap buttons at
	// the foot work as a remote.
	import type { Snippet } from 'svelte';
	import { ballPosition, changeEndsLine, pointButtonLabel } from '#lib/fence.ts';
	import { strings } from '#lib/strings.ts';
	import type { Team } from '#lib/types.ts';
	import type { FenceModel, SetDetail } from '#lib/view.ts';
	import TeamNames from './TeamNames.svelte';

	let {
		model,
		onpoint,
		meta,
		undo,
		banner
	}: {
		model: FenceModel;
		/** Absent on the spectator view: no tap buttons, no undo. */
		onpoint?: (team: Team) => void;
		meta: Snippet;
		undo?: Snippet;
		banner?: Snippet;
	} = $props();

	const f = strings.fence;
	const timed = $derived(model.a.sets.length === 0);
	// The serve ball sits on the serving team's points box, at the corner of the side served from.
	const ball = $derived(ballPosition(model));

	interface Column {
		label: string;
		a: SetDetail;
		b: SetDetail;
	}

	// One column per set; the super tie-break in progress has none, its points are the big
	// figure. A timed match has one column, its games.
	const columns = $derived.by((): Column[] => {
		if (timed) {
			const g = (games: number): SetDetail => ({ games, tiebreak: null, superTieBreak: false, won: null });
			return [{ label: f.games, a: g(model.a.games), b: g(model.b.games) }];
		}
		const out: Column[] = [];
		model.a.setDetails.forEach((a, i) => {
			const b = model.b.setDetails[i];
			if (!b || (a.superTieBreak && a.won === null)) return;
			out.push({ label: f.set(i + 1), a, b });
		});
		return out;
	});
</script>

{#snippet setCell(d: SetDetail)}
	<b class="num" class:lost={d.won === false}>
		{#if d.superTieBreak && d.tiebreak !== null}
			{d.tiebreak}
		{:else}
			{d.games}{#if d.tiebreak !== null}<sup>{d.tiebreak}</sup>{/if}
		{/if}
	</b>
{/snippet}

<div class="board" class:done={model.finished !== null}>
	<div class="meta">
		<div class="meta-left">{@render meta()}</div>
		<span class="rules">{model.rulesLine}</span>
	</div>

	{#each ['a', 'b'] as const as t (t)}
		{@const team = model[t]}
		<section class="tb {t}" aria-label={team.name}>
			<div class="name">
				<span class="team-name"><TeamNames {team} /></span>
				{#if team.serving && model.serveSide}<small>{f.servingFrom[model.serveSide]}</small>{/if}
			</div>
			<div class="nums" style:--cols={columns.length}>
				{#each columns as col (col.label)}
					<div class="col">
						<span>{col.label}</span>
						{@render setCell(col[t])}
					</div>
				{/each}
				{#if !model.finished}
					<div class="p num" class:clear={ball?.team === t && ball.side === 'centre'}>
						{team.points}
						{#if ball && ball.team === t}<span class="ball {ball.side}" aria-hidden="true"></span>{/if}
					</div>
				{/if}
			</div>
		</section>
	{/each}

	{#if model.changeEnds && !model.finished}
		<p class="changeends" role="status">{changeEndsLine(model)}</p>
	{:else if model.callout}
		<p class="callout" role="status">{model.callout}</p>
	{/if}

	{#if banner}
		<div class="banner">{@render banner()}</div>
	{/if}

	{#if onpoint}
		<div class="foot">
			{@render undo?.()}
			{#if !model.finished}
				<button type="button" class="btn a" onclick={() => onpoint('a')}>
					{pointButtonLabel(model.a)}
				</button>
				<button type="button" class="btn b" onclick={() => onpoint('b')}>
					{pointButtonLabel(model.b)}
				</button>
			{/if}
		</div>
	{/if}
</div>

<style>
	.board {
		display: flex;
		flex-direction: column;
		gap: clamp(10px, 2vh, 18px);
		min-height: 100%;
		padding: clamp(12px, 2.6vh, 22px) 16px max(16px, env(safe-area-inset-bottom));
		background: var(--glass);
	}
	.meta {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 8px;
		font-size: 13px;
		color: var(--muted);
	}
	.meta-left {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.rules {
		text-align: right;
	}
	.tb {
		flex: 1 1 0;
		display: flex;
		flex-direction: column;
		min-height: 0;
		border-radius: 18px;
		padding: 14px 14px 12px;
		background: var(--panel);
		border: 1px solid var(--line);
	}
	.done .tb {
		flex: none;
	}
	.tb.a {
		border-left: 8px solid var(--a);
	}
	.tb.b {
		border-left: 8px solid var(--b);
	}
	.name {
		display: flex;
		align-items: center;
		gap: 8px;
		font-weight: 650;
		font-size: clamp(18px, 2.6vh, 26px);
		line-height: 1.2;
	}
	.team-name {
		overflow-wrap: anywhere;
	}
	.name small {
		margin-left: auto;
		font-weight: 400;
		color: var(--muted);
		font-size: clamp(13px, 1.8vh, 18px);
		text-align: right;
	}
	.nums {
		flex: 1;
		display: grid;
		grid-template-columns: repeat(var(--cols), minmax(40px, auto)) 1fr;
		gap: 8px;
		align-items: end;
		margin-top: 8px;
	}
	.col {
		text-align: center;
	}
	.col span {
		display: block;
		font-size: 11px;
		color: var(--muted);
		margin-bottom: 4px;
		white-space: nowrap;
	}
	.col b {
		font-size: clamp(40px, 6vh, 64px);
	}
	.col b.lost {
		color: var(--muted);
	}
	.col sup {
		font-size: 0.42em;
		vertical-align: top;
		margin-left: 1px;
	}
	.p {
		/* 118px on the mockup's 620px-high phone (19%), here as large as the block allows. */
		align-self: stretch;
		display: flex;
		align-items: center;
		justify-content: center;
		font-size: clamp(72px, min(21vh, 44vw), 320px);
		color: var(--on-team);
		border-radius: 14px;
		padding: 12px 0 8px;
		position: relative;
	}
	/*
	 * The serve ball, as on the court, inside a corner of the points box, clear of the figures.
	 * Team A's box is the top one, so its ball sits in a top corner; team B's in a bottom one.
	 * Right or left as served; the middle when the receiver picks, where the box makes room for
	 * it above and below so the figures stay put.
	 */
	.p {
		--ball-size: clamp(20px, 4.4vh, 36px);
		--ball-inset: 7px;
	}
	.p.clear {
		padding-block: calc(var(--ball-size) + 2 * var(--ball-inset));
	}
	.ball {
		position: absolute;
		width: var(--ball-size);
		height: var(--ball-size);
		border-radius: 50%;
		background: var(--ball);
		box-shadow:
			inset -3px -3px 0 rgba(0, 0, 0, 0.12),
			0 0 0 3px rgba(255, 255, 255, 0.35);
	}
	.a .ball {
		top: var(--ball-inset);
	}
	.b .ball {
		bottom: var(--ball-inset);
	}
	.ball.right {
		right: var(--ball-inset);
	}
	.ball.left {
		left: var(--ball-inset);
	}
	.ball.centre {
		left: 50%;
		transform: translateX(-50%);
	}
	.a .p {
		background: var(--a);
	}
	.b .p {
		background: var(--b);
	}
	.changeends {
		margin: 0;
		border: 2px dashed var(--line);
		border-radius: 12px;
		padding: 10px 12px;
		font-size: 14px;
		color: var(--muted);
	}
	.callout {
		margin: 0;
		align-self: center;
		background: var(--ball);
		color: var(--on-ball);
		font-weight: 750;
		font-size: 15px;
		padding: 4px 14px;
		border-radius: 999px;
	}
	.foot {
		display: flex;
		gap: 10px;
	}
	.btn {
		flex: 1;
		min-height: 56px;
		border: 0;
		border-radius: 14px;
		padding: 14px 8px;
		font-weight: 650;
		color: var(--on-team);
		touch-action: manipulation;
		overflow-wrap: anywhere;
	}
	.btn.a {
		background: var(--a);
	}
	.btn.b {
		background: var(--b);
	}
	.btn:active {
		filter: brightness(1.08);
	}
</style>
