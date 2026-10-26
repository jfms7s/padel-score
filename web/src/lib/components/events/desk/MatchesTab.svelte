<!--
	The desk's Matches tab: every match with its court, result entry and View, and the New
	match form while the event runs.
-->
<script lang="ts">
	import TypedScoreSheet from '#lib/components/TypedScoreSheet.svelte';
	import { liveScoreText, matchText, matchViewPath, teamText } from '#lib/events.ts';
	import { stickyClientId } from '#lib/ids.ts';
	import { scoreText } from '#lib/players.ts';
	import { MAX_NAME_LENGTH } from '#lib/setup.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventDetail, EventMatch, NewEventMatch, Teams, TypedScore } from '#lib/types.ts';

	let {
		detail,
		matches,
		busy,
		failure,
		onfail,
		oncreate,
		onmove,
		onsave
	}: {
		detail: EventDetail;
		/** The matches in their order. */
		matches: EventMatch[];
		busy: boolean;
		failure: string | undefined;
		onfail: (message: string) => void;
		oncreate: (body: NewEventMatch) => Promise<boolean>;
		onmove: (m: EventMatch, courtId: string) => void;
		/** Throws to refuse; the sheet explains it in place. */
		onsave: (m: EventMatch, score: TypedScore, clientId: string) => Promise<void>;
	} = $props();

	const t = strings.events;

	const ev = $derived(detail.event);
	const status = $derived(ev.status);
	const teams = $derived(detail.teams);
	const courts = $derived(detail.courts);

	let teamA = $state('');
	let teamB = $state('');
	let matchCourt = $state('');
	let matchLabel = $state('');
	let sheetFor = $state<string | null>(null);
	/** One client id per typed score while a sheet is open, so a retry is replayed. */
	const resultIds = stickyClientId();

	const courtName = (cid: string | null) => courts.find((c) => c.id === cid)?.name ?? t.notOnCourt;

	function openSheet(match: EventMatch) {
		resultIds.reset();
		sheetFor = match.id;
	}

	async function submitMatch(event: SubmitEvent) {
		event.preventDefault();
		if (teamA === '' || teamB === '') {
			onfail(t.pickTeam);
			return;
		}
		if (teamA === teamB) {
			onfail(t.sameTeam);
			return;
		}
		const label = matchLabel.trim();
		const made = await oncreate({
			team_a: teamA,
			team_b: teamB,
			...(matchCourt === '' ? {} : { court_id: matchCourt }),
			...(label === '' ? {} : { label })
		});
		if (made) {
			teamA = '';
			teamB = '';
			matchCourt = '';
			matchLabel = '';
		}
	}

	/** The players' names as the sheet shows them. */
	function teamsOf(match: EventMatch): Teams {
		const names = (tid: string): [string, string] => {
			const tm = teams.find((x) => x.id === tid);
			return tm ? [tm.players[0].name, tm.players[1].name] : [t.unknownTeam, t.unknownTeam];
		};
		return { a: names(match.team_a), b: names(match.team_b) };
	}

	async function saveResult(match: EventMatch, score: TypedScore) {
		// Throws to the sheet, which explains a refusal in place.
		await onsave(match, score, resultIds.for(score));
		sheetFor = null;
	}

	function resultText(m: EventMatch): string | null {
		if (m.state === 'finished' && m.result) {
			const score = scoreText(m.result);
			if (m.winner === null) return `${score} · ${t.draw}`;
			const winner = teams.find((x) => x.id === (m.winner === 'a' ? m.team_a : m.team_b));
			return winner ? `${score} · ${strings.events.sheet.winner(teamText(winner))}` : score;
		}
		if (m.state === 'live' && m.live) return liveScoreText(m.live);
		return null;
	}
</script>

<section aria-labelledby="h-matches">
	<h2 id="h-matches">{t.matches}</h2>
	{#if matches.length === 0}
		<p class="hint">{t.noMatches}</p>
	{:else}
		<ul class="list" aria-labelledby="h-matches">
			{#each matches as m (m.id)}
				{@const text = matchText(m, teams)}
				{@const result = resultText(m)}
				<li class="row match">
					<span class="what">
						<strong>{m.label}</strong>
						<span>{text}</span>
						<span class="hint">
							{t.matchState[m.state]}{#if m.locked}{' · '}{t.enteredByYou}{/if}{#if result}{' · '}{result}{/if}
						</span>
					</span>
					<div class="actions">
						{#if status === 'running' && m.state !== 'finished'}
							<label class="court-pick">
								<span class="visually-hidden">{t.moveLabel(text)}</span>
								<select value={m.court_id ?? ''} disabled={busy} onchange={(e) => onmove(m, e.currentTarget.value)}>
									<option value="">{t.notOnCourt}</option>
									{#each courts as c (c.id)}
										<option value={c.id}>{c.name}</option>
									{/each}
								</select>
							</label>
						{:else}
							<span class="hint court-name">{courtName(m.court_id)}</span>
						{/if}
						{#if status === 'running'}
							<button class="secondary" type="button" disabled={busy} aria-label={t.enterResultLabel(text)} onclick={() => openSheet(m)}>
								{t.enterResult}
							</button>
						{/if}
						<a class="secondary" href={matchViewPath(m.spectator_token)} aria-label={t.viewLabel(text)}>{t.view}</a>
					</div>
					{#if sheetFor === m.id}
						<TypedScoreSheet
							format={ev.format}
							teams={teamsOf(m)}
							title={t.sheet.title(text)}
							onsubmit={(score) => saveResult(m, score)}
							oncancel={() => (sheetFor = null)}
						/>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}

	{#if status === 'running'}
		<h3>{t.newMatch}</h3>
		<form class="new-match" onsubmit={submitMatch}>
			<label>
				{t.teamA}
				<select bind:value={teamA} required>
					<option value="" disabled>{t.pickTeam}</option>
					{#each teams as tm (tm.id)}<option value={tm.id}>{teamText(tm)}</option>{/each}
				</select>
			</label>
			<label>
				{t.teamB}
				<select bind:value={teamB} required>
					<option value="" disabled>{t.pickTeam}</option>
					{#each teams as tm (tm.id)}<option value={tm.id}>{teamText(tm)}</option>{/each}
				</select>
			</label>
			<label>
				{t.court}
				<select bind:value={matchCourt}>
					<option value="">{t.notOnCourt}</option>
					{#each courts as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
				</select>
			</label>
			<label>
				{t.label}
				<input bind:value={matchLabel} autocomplete="off" maxlength={MAX_NAME_LENGTH} />
			</label>
			<button class="primary" type="submit" disabled={busy}>{t.create_match}</button>
		</form>
	{/if}
	{#if failure}<p class="error" role="alert">{failure}</p>{/if}
</section>

<style>
	section {
		margin-top: 8px;
	}
	h3 {
		margin: 20px 0 8px;
		font-size: 16px;
		font-weight: 700;
	}
	.row .what {
		flex: 1 1 10rem;
	}
	select {
		min-height: var(--tap);
		width: 100%;
		padding: 0 10px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		font-size: 16px;
	}
	.match .actions {
		align-items: center;
	}
	.court-pick {
		flex: 1 1 10rem;
	}
	.court-name {
		align-self: center;
	}
	p.error {
		margin-top: 12px;
	}
</style>
