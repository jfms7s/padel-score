<!--
	A court seen from above: the live score as two halves around a net, the serve ball in the
	server's half. Finished: faded halves and a bar with the next match. Idle: a quiet panel.
-->
<script lang="ts">
	import { courtTile } from '#lib/desk.ts';
	import { strings } from '#lib/strings.ts';
	import type { CourtOut, EventMatch, EventTeam, EventViewCourt, EventViewMatch } from '#lib/types.ts';

	let {
		court,
		matches,
		teams,
		onnext,
		busy = false,
		compact = false
	}: {
		court: CourtOut | EventViewCourt;
		matches: (EventMatch | EventViewMatch)[];
		teams: EventTeam[];
		onnext?: () => void;
		busy?: boolean;
		compact?: boolean;
	} = $props();

	const t = strings.desk;
	const tile = $derived(courtTile(court, matches, teams));
	const sides = $derived(tile.state === 'idle' ? [] : ([['a', tile.a], ['b', tile.b]] as const));
	const phones = $derived('phones' in court ? court.phones : null);
</script>

<article class="court" class:done={tile.state === 'finished'} class:idle={tile.state === 'idle'} class:compact role="group" aria-label={court.name}>
	<div class="head">
		<b>{court.name}</b>
		{#if tile.state === 'live'}<span class="chip live">{t.live}</span>{/if}
		{#if tile.state === 'finished'}<span class="chip">{t.finished}</span>{/if}
	</div>
	{#if tile.state !== 'idle'}<div class="label">{tile.label}</div>{/if}

	<div class="pitch">
		{#if tile.state === 'idle'}
			<span class="none">{t.idle}</span>
		{:else}
			{#each sides as [team, s], i (team)}
				{#if i === 1}<div class="net" aria-hidden="true"></div>{/if}
				<div class="s {team}">
					<span class="who">{s.names}</span>
					{#if tile.state === 'live'}
						<span class="pts num">{s.point}</span>
						<span class="gm">
							{#if s.sets !== null}{t.sets}<b>{s.sets}</b> · {/if}{t.games}<b>{s.games}</b>
						</span>
						{#if tile.server === team}
							<span class="ball"><span class="visually-hidden">{t.serving(s.names)}</span></span>
						{/if}
					{:else}
						<span class="pts num fin">{s.games}</span>
						<span class="gm">{#if tile.winner === team}{t.won}{:else}&nbsp;{/if}</span>
					{/if}
				</div>
			{/each}
		{/if}
	</div>

	{#if tile.state === 'finished'}
		{#if tile.nextLabel !== null || onnext}
			<div class="foot next">
				<span class="grow">{tile.nextLabel !== null ? t.next(tile.nextLabel) : ''}</span>
				{#if onnext}<button class="nextbtn" type="button" disabled={busy} onclick={onnext}>{t.nextMatch}</button>{/if}
			</div>
		{/if}
	{:else}
		<div class="foot">
			{#if phones !== null}<span class="grow">{t.phones(phones)}</span>{:else}<span class="grow"></span>{/if}
			<a class="watch" href="/cv/{encodeURIComponent(court.spectator_token)}" aria-label={strings.court.watchLabel(court.name)}>{t.watch}</a>
		</div>
	{/if}
</article>

<style>
	.court {
		display: flex;
		flex-direction: column;
		min-width: 0;
		overflow: hidden;
		border-radius: 16px;
		border: 1px solid var(--line);
		background: var(--panel);
	}
	.head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 10px 12px 8px;
	}
	.head b {
		font-variation-settings: 'wdth' 80;
		font-size: 17px;
		font-weight: 800;
	}
	.chip {
		margin-left: auto;
		display: inline-flex;
		align-items: center;
		gap: 6px;
		padding: 2px 10px;
		border-radius: 999px;
		font-size: 13px;
		font-weight: 650;
		white-space: nowrap;
		background: var(--a-soft);
		color: var(--ink);
	}
	.chip.live::before {
		content: '';
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--ball);
		box-shadow: 0 0 0 2px rgba(217, 234, 42, 0.3);
	}
	.label {
		margin-top: -4px;
		padding: 0 12px 8px;
		font-size: 13px;
		color: var(--muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.pitch {
		position: relative;
		display: grid;
		grid-template-columns: 1fr 6px 1fr;
		min-height: 112px;
		color: #fff;
	}
	.compact .pitch {
		min-height: 84px;
	}
	.s {
		position: relative;
		display: flex;
		flex-direction: column;
		justify-content: space-between;
		min-width: 0;
		padding: 10px 12px;
		overflow: hidden;
	}
	.s.a {
		background: var(--a);
	}
	.s.b {
		background: var(--b);
		text-align: right;
		align-items: flex-end;
	}
	/* Service lines: the line across the court and the centre line. */
	.s::before {
		content: '';
		position: absolute;
		top: 0;
		bottom: 0;
		width: 2px;
		background: rgba(255, 255, 255, 0.4);
	}
	.s.a::before {
		right: 30%;
	}
	.s.b::before {
		left: 30%;
	}
	.s::after {
		content: '';
		position: absolute;
		top: 50%;
		height: 2px;
		width: 30%;
		background: rgba(255, 255, 255, 0.4);
	}
	.s.a::after {
		right: 0;
	}
	.s.b::after {
		left: 0;
	}
	.net {
		background: repeating-linear-gradient(0deg, #fff 0 3px, #c7d2da 3px 6px);
	}
	.who {
		position: relative;
		z-index: 1;
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-weight: 650;
		font-size: 14px;
	}
	.pts {
		position: relative;
		z-index: 1;
		font-size: 54px;
	}
	.compact .pts {
		font-size: 40px;
	}
	.pts.fin {
		font-size: 34px;
	}
	.compact .pts.fin {
		font-size: 28px;
	}
	.gm {
		position: relative;
		z-index: 1;
		font-size: 13px;
		opacity: 0.95;
	}
	.gm b {
		margin-left: 3px;
		font-variation-settings: 'wdth' 62;
		font-weight: 900;
		font-size: 20px;
	}
	.ball {
		position: absolute;
		z-index: 2;
		width: 14px;
		height: 14px;
		border-radius: 50%;
		background: var(--ball);
		box-shadow: 0 0 0 3px rgba(255, 255, 255, 0.35);
	}
	.a .ball {
		left: 58%;
		top: 52%;
	}
	.b .ball {
		right: 58%;
		top: 30%;
	}
	.done .s {
		opacity: 0.55;
	}
	.idle .pitch {
		grid-template-columns: 1fr;
		place-items: center;
		background: var(--glass);
		color: var(--muted);
		font-size: 14px;
	}
	.none {
		padding: 10px 12px;
		text-align: center;
	}
	.foot {
		display: flex;
		align-items: center;
		gap: 8px;
		min-height: 44px;
		padding: 8px 12px;
		border-top: 1px solid var(--line);
		font-size: 13px;
		color: var(--muted);
	}
	.grow {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.foot.next {
		background: var(--ball);
		color: #0b2236;
		font-weight: 650;
	}
	.watch {
		display: inline-flex;
		align-items: center;
		min-height: 32px;
		padding: 0 12px;
		border-radius: 999px;
		border: 1px solid var(--line);
		color: var(--ink);
		font-weight: 650;
		text-decoration: none;
	}
	.nextbtn {
		min-height: 36px;
		padding: 0 14px;
		border: 0;
		border-radius: 999px;
		background: #0b2236;
		color: #fff;
		font-weight: 700;
	}
	.nextbtn:disabled {
		opacity: 0.6;
		cursor: default;
	}
</style>
