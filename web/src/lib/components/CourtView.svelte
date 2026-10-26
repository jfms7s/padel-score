<script lang="ts">
	// The fence screen sideways: the court seen from above, team A's half on the left. Each half
	// is one big button. The white lines are the service line and the centre service line; the
	// ball sits behind the server's baseline on the side they serve from.
	import type { Snippet } from 'svelte';
	import { ballPosition } from '#lib/fence.ts';
	import { strings } from '#lib/strings.ts';
	import type { Team } from '#lib/types.ts';
	import type { FenceModel } from '#lib/view.ts';

	let {
		model,
		onpoint,
		controls,
		banner
	}: {
		model: FenceModel;
		/** Absent on the spectator view: the halves are then plain, not tappable. */
		onpoint?: (team: Team) => void;
		controls?: Snippet;
		banner?: Snippet;
	} = $props();

	const ball = $derived(ballPosition(model));
	const timed = $derived(model.a.sets.length === 0);
	const f = strings.fence;

	/**
	 * Vertical place of the ball. Team A faces right (towards the net), so its right is the
	 * bottom of the screen; team B faces left, so its right is the top.
	 */
	function ballTop(team: Team, side: 'right' | 'left' | 'centre'): string {
		if (side === 'centre') return '50%';
		const low = (team === 'a') === (side === 'right');
		return low ? '68%' : '32%';
	}
</script>

<div class="court">
	{#each ['a', 'b'] as const as t (t)}
		{@const team = model[t]}
		<svelte:element
			this={onpoint ? 'button' : 'div'}
			type={onpoint ? 'button' : undefined}
			class="side {t}"
			role={onpoint ? undefined : 'group'}
			aria-label={onpoint ? f.pointTo(team.name) : team.name}
			disabled={onpoint ? model.finished !== null : undefined}
			onclick={onpoint ? () => onpoint(t) : undefined}
		>
			<span class="box-line" aria-hidden="true"></span>
			<span class="team">
				{team.name}
				{#if team.serving && model.servingPlayerName}
					<span class="server">{f.playerServing(model.servingPlayerName)}</span>
				{/if}
			</span>
			<span class="pts num" aria-hidden={team.points === ''}>{team.points}</span>
			<span class="tally">
				{f.games} <b>{team.games}</b>
				{#if !timed}{f.sets} <b>{team.setsWon}</b>{/if}
			</span>
			{#if ball && ball.team === t}
				<span class="ball" style:top={ballTop(t, ball.side)}></span>
			{/if}
		</svelte:element>
		{#if t === 'a'}<div class="net" aria-hidden="true"></div>{/if}
	{/each}

	{#if model.callout}
		<span class="callout" role="status">{model.callout}</span>
	{:else if model.changeEnds}
		<span class="callout ends" role="status">{strings.changeEnds.short}</span>
	{/if}

	{#if controls}<div class="controls">{@render controls()}</div>{/if}

	{#if banner}<div class="banner">{@render banner()}</div>{/if}
</div>

<style>
	.court {
		position: relative;
		display: grid;
		grid-template-columns: 1fr clamp(8px, 1.4vw, 16px) 1fr;
		height: 100%;
		background: var(--glass);
	}
	.side {
		position: relative;
		display: flex;
		align-items: center;
		justify-content: center;
		min-height: 0;
		margin: 0;
		padding: 0;
		border: 0;
		border-radius: 0;
		color: var(--on-team);
		overflow: hidden;
		touch-action: manipulation;
		user-select: none;
		-webkit-user-select: none;
		-webkit-tap-highlight-color: transparent;
	}
	.side.a {
		background: var(--a);
	}
	.side.b {
		background: var(--b);
	}
	button.side:active:not(:disabled) {
		filter: brightness(1.08);
	}
	.side:disabled {
		cursor: default;
	}
	.side:focus-visible {
		outline: 4px solid var(--ball);
		outline-offset: -8px;
	}
	/* The service line, 30% of the half from the net. */
	.side::before {
		content: '';
		position: absolute;
		top: 0;
		bottom: 0;
		width: 2px;
		background: rgba(255, 255, 255, 0.55);
	}
	.side.a::before {
		right: 30%;
	}
	.side.b::before {
		left: 30%;
	}
	/* The centre service line, from the service line to the net. */
	.box-line {
		position: absolute;
		top: 50%;
		height: 2px;
		width: 30%;
		background: rgba(255, 255, 255, 0.55);
	}
	.a .box-line {
		right: 0;
	}
	.b .box-line {
		left: 0;
	}
	.net {
		background: repeating-linear-gradient(0deg, #fff 0 3px, #c7d2da 3px 6px);
	}
	.team {
		position: absolute;
		top: clamp(10px, 3.6vh, 24px);
		left: clamp(14px, 2.2vw, 28px);
		max-width: 62%;
		font-size: clamp(16px, 2.4vw, 30px);
		font-weight: 600;
		opacity: 0.92;
		text-align: left;
		line-height: 1.2;
		overflow-wrap: anywhere;
	}
	/* The serving player's name, under the team's and as large, a little bolder. */
	.server {
		display: block;
		margin-top: 0.15em;
		font-weight: 750;
	}
	.b .team {
		left: auto;
		right: clamp(14px, 2.2vw, 28px);
		text-align: right;
	}
	.pts {
		/* 168px on the mockup's 300px-high phone: 56% of the height, capped by the width. */
		font-size: clamp(64px, min(58vh, 25vw), 520px);
		letter-spacing: -0.02em;
	}
	.tally {
		position: absolute;
		bottom: clamp(10px, 3.6vh, 24px);
		left: clamp(14px, 2.2vw, 28px);
		display: flex;
		align-items: baseline;
		gap: 0.4em;
		font-size: clamp(16px, 2.6vw, 34px);
		white-space: nowrap;
	}
	.b .tally {
		left: auto;
		right: clamp(14px, 2.2vw, 28px);
	}
	.tally b {
		font-variation-settings: 'wdth' 62;
		font-weight: 800;
		font-size: 2em;
		line-height: 1;
		margin-right: 0.5em;
	}
	.tally b:last-child {
		margin-right: 0;
	}
	.ball {
		position: absolute;
		width: clamp(18px, 7.3vh, 44px);
		height: clamp(18px, 7.3vh, 44px);
		border-radius: 50%;
		background: var(--ball);
		box-shadow:
			inset -3px -3px 0 rgba(0, 0, 0, 0.12),
			0 0 0 3px rgba(255, 255, 255, 0.35);
		transform: translateY(-50%);
	}
	.a .ball {
		left: 4%;
	}
	.b .ball {
		right: 4%;
	}
	.callout {
		position: absolute;
		left: 50%;
		top: clamp(8px, 3vh, 20px);
		transform: translateX(-50%);
		background: var(--ball);
		color: var(--on-ball);
		font-weight: 750;
		font-size: clamp(14px, 2.4vw, 26px);
		padding: 0.3em 0.9em;
		border-radius: 999px;
		white-space: nowrap;
		pointer-events: none;
	}
	.callout.ends {
		background: var(--ink);
		color: var(--glass);
	}
	.controls {
		position: absolute;
		left: 50%;
		bottom: clamp(6px, 2.4vh, 16px);
		transform: translateX(-50%);
		display: grid;
		grid-template-columns: 1fr auto 1fr;
		align-items: center;
		gap: 8px;
		width: min(100% - 32px, 26rem);
		pointer-events: none;
	}
	.controls :global(button) {
		pointer-events: auto;
	}
	.banner {
		position: absolute;
		left: 50%;
		top: 50%;
		transform: translate(-50%, -50%);
		width: min(90%, 30rem);
	}
</style>
