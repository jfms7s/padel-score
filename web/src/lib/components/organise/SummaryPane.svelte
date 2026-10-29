<script lang="ts">
	// The Organise page's right-hand pane: the selected row in full, with its share and open actions.
	import { longDate } from '#lib/dates.ts';
	import { rulesText } from '#lib/events.ts';
	import type { OrganiseRow } from '#lib/organise.ts';
	import { shareLink } from '#lib/share.ts';
	import { joinLink } from '#lib/signups.ts';
	import { strings } from '#lib/strings.ts';
	import type { OrganisedBoard, OrganisedEvent, OrganisedMatch } from '#lib/types.ts';

	let {
		row,
		data
	}: { row: OrganiseRow; data: OrganisedEvent | OrganisedMatch | OrganisedBoard } = $props();

	const t = strings.organise;
	const enc = encodeURIComponent;

	let note = $state<string | null>(null);

	const ev = $derived(row.kind === 'event' ? (data as OrganisedEvent) : null);
	const match = $derived(row.kind === 'match' ? (data as OrganisedMatch) : null);
	const board = $derived(row.kind === 'board' ? (data as OrganisedBoard) : null);

	const signupsOn = $derived(ev !== null && ev.schedule.signups !== 'off');
	const deskHref = $derived(
		ev === null ? row.href : `/events/${enc(ev.id)}${ev.status === 'setup' && signupsOn ? '?tab=signups' : ''}`
	);
	const details = $derived(
		[row.when, row.place, ev ? rulesText(ev.format) : null].filter((p): p is string => p !== null && p !== '').join(' · ')
	);
	const endDay = $derived(
		board !== null && longDate(board.ends_at) !== longDate(board.starts_at) ? longDate(board.ends_at) : null
	);
	const shareUrl = $derived.by(() => {
		if (ev !== null && ev.join_token !== null && signupsOn) return joinLink(location.origin, ev.join_token);
		if (match !== null) return joinLink(location.origin, match.join_token);
		if (board !== null) return `${location.origin}/b/${enc(board.token)}`;
		return null;
	});
	const shareLabel = $derived(row.kind === 'event' ? t.share : t.shareLink);
	const barMax = $derived(ev === null ? 1 : Math.max(ev.schedule.max_teams ?? ev.signed_teams, 1));

	// A new selection starts without the last one's message.
	$effect(() => {
		void row.key;
		note = null;
	});

	async function share() {
		if (shareUrl === null) return;
		note = null;
		const result = await shareLink(shareUrl, strings.signups.shareTitle(row.title));
		if (result === 'copied') note = strings.share.linkCopied;
		else if (result === 'failed') note = strings.share.linkFailed;
	}
</script>

<section class="pane" aria-label={t.summaryLabel}>
	<span class="chip {row.kind}">{ev ? strings.events.kinds[ev.kind] : row.kind === 'match' ? t.match : t.board}</span>
	<h2>{row.title}</h2>
	<p class="hint details">{details}</p>

	{#if ev && signupsOn}
		<div class="card">
			<div
				class="bar"
				role="progressbar"
				aria-label={t.signups}
				aria-valuemin={0}
				aria-valuemax={barMax}
				aria-valuenow={ev.signed_teams}
			>
				<span style:width="{Math.min(100, (ev.signed_teams / barMax) * 100)}%"></span>
			</div>
			<p>{t.signupsText(ev.signed_teams, ev.solo_count, ev.line_count)}</p>
			{#if ev.schedule.deadline_at !== null && ev.schedule.deadline_at > Date.now() / 1000}
				<p class="hint">{t.signupsClose(longDate(ev.schedule.deadline_at))}</p>
			{/if}
		</div>
	{/if}

	{#if match}
		<ul class="places">
			{#each match.places as place, i (i)}
				<li class:open={place === null}>{place ?? t.placeOpen}</li>
			{/each}
		</ul>
		{#if match.line_count > 0}<p>{t.inLine(match.line_count)}</p>{/if}
	{/if}

	{#if board}
		<p>{t.boardStats(board.member_count, board.game_count)}</p>
		{#if endDay}<p class="hint">{endDay}</p>{/if}
	{/if}

	{#if row.needs}
		<p class="needs"><span class="hint">{t.needsYou}</span> {row.needs}</p>
	{/if}

	<div class="actions">
		{#if shareUrl !== null}
			<button class="secondary" type="button" onclick={share}>{shareLabel}</button>
		{/if}
		<a class="primary" href={deskHref}>{ev ? t.openDesk : t.open}</a>
	</div>
	{#if note}<p class="hint" role="status">{note}</p>{/if}
</section>

<style>
	.pane {
		min-width: 0;
		padding: 16px;
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--panel);
	}
	.pane h2 {
		margin-top: 8px;
		overflow-wrap: anywhere;
	}
	.details {
		margin-bottom: 16px;
	}
	.chip {
		display: inline-block;
		padding: 0 8px;
		border-radius: 999px;
		border: 1px solid currentColor;
		font-size: 12px;
		font-weight: 650;
	}
	.chip.event {
		color: var(--ev);
	}
	.chip.match {
		color: var(--b);
	}
	.chip.board {
		color: var(--a);
	}
	.card {
		margin-bottom: 12px;
		padding: 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
	}
	.bar {
		height: 8px;
		margin-bottom: 10px;
		border-radius: 999px;
		background: var(--line);
		overflow: hidden;
	}
	.bar span {
		display: block;
		height: 100%;
		background: var(--ev);
	}
	.places {
		margin: 0 0 12px;
		padding: 0;
		list-style: none;
	}
	.places li {
		padding: 6px 0;
		border-bottom: 1px solid var(--line);
	}
	.places li.open {
		color: var(--muted);
	}
	.needs {
		color: var(--ink);
		font-weight: 650;
	}
	.needs .hint {
		font-weight: 400;
	}
	.actions {
		margin-top: 16px;
	}
</style>
