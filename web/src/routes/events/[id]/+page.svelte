<script lang="ts">
	// The coordinator's event screen: teams, courts with their QR codes, matches with typed
	// results, the kind's own panel, then start and finish. It follows the event's stream and
	// offers a reload when the stream is lost.
	import { onDestroy, onMount } from 'svelte';
	import { page } from '$app/state';
	import PlayerPicker from '#lib/components/PlayerPicker.svelte';
	import QrCode from '#lib/components/QrCode.svelte';
	import TypedScoreSheet from '#lib/components/TypedScoreSheet.svelte';
	import KindPanel from '#lib/components/events/KindPanel.svelte';
	import {
		addCourt,
		addTeam,
		advanceCourt,
		assignMatch,
		courtLinks,
		courtStatusText,
		createEventMatch,
		deleteTeam,
		eventLink,
		eventMessage,
		eventStreamUrl,
		finishEvent,
		getEvent,
		groupsFromTies,
		isEventDetail,
		liveScoreText,
		matchText,
		matchViewPath,
		moveItem,
		placesFromOrder,
		postEventResult,
		rotateCourt,
		rulesText,
		startEvent,
		teamText,
		updateTeam
	} from '#lib/events.ts';
	import { newClientId } from '#lib/ids.ts';
	import { listPlayers, scoreText } from '#lib/players.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { MAX_NAME_LENGTH } from '#lib/setup.ts';
	import { openSseStream, type StreamStatus } from '#lib/stream.ts';
	import { strings } from '#lib/strings.ts';
	import type { CourtOut, EventDetail, EventMatch, EventTeam, Player, Teams, TypedScore } from '#lib/types.ts';

	const t = strings.events;

	const id = $derived(page.params.id ?? '');
	const origin = $derived(page.url.origin);

	let detail = $state<EventDetail | null>(null);
	let loadFailure = $state<string | null>(null);
	let streamStatus = $state<StreamStatus | null>(null);
	let stream: { close(): void } | null = null;

	/** The signed-in user's players, for adding teams; null until loaded or if unreadable. */
	let players = $state<Player[] | null>(null);
	let playersFailed = $state(false);

	type Section = 'teams' | 'courts' | 'matches' | 'lifecycle';
	let busy = $state(false);
	let failures = $state<Partial<Record<Section, string>>>({});

	const ev = $derived(detail?.event ?? null);
	const status = $derived(ev?.status ?? 'setup');
	const teams = $derived(detail?.teams ?? []);
	const courts = $derived(detail?.courts ?? []);
	const matches = $derived([...(detail?.matches ?? [])].sort((x, y) => x.ordinal - y.ordinal));

	onMount(() => {
		void load();
	});
	onDestroy(() => stream?.close());

	async function load() {
		const s = await requireSignedIn();
		if (s.status === 'unavailable') loadFailure = strings.networkError;
		if (s.status !== 'signed_in') return;
		try {
			detail = await getEvent(id);
		} catch (e) {
			loadFailure = eventMessage(e);
			return;
		}
		stream = openSseStream(
			eventStreamUrl(id),
			{ type: 'event', accept: isEventDetail },
			(d) => (detail = d),
			(st) => (streamStatus = st)
		);
		try {
			players = await listPlayers({ quiet: true });
		} catch {
			playersFailed = true;
		}
	}

	async function refresh() {
		detail = await getEvent(id);
	}

	/** Run one action at a time; its failure shows in its own section. */
	async function act(section: Section, run: () => Promise<void>): Promise<boolean> {
		if (busy) return false;
		busy = true;
		failures = {};
		try {
			await run();
			return true;
		} catch (e) {
			failures = { [section]: eventMessage(e) };
			return false;
		} finally {
			busy = false;
		}
	}

	// ---- copying links ---------------------------------------------------------------------

	let copied = $state<string | null>(null);
	let copyFailed = $state(false);

	async function copy(key: string, link: string) {
		copyFailed = false;
		try {
			await navigator.clipboard.writeText(link);
			copied = key;
			setTimeout(() => {
				if (copied === key) copied = null;
			}, 2000);
		} catch {
			copyFailed = true;
		}
	}

	// ---- teams -----------------------------------------------------------------------------

	let first = $state({ name: '', id: null as string | null });
	let second = $state({ name: '', id: null as string | null });
	let teamName = $state('');
	let renaming = $state<string | null>(null);
	let renameText = $state('');

	const inTeams = $derived(teams.flatMap((tm) => tm.players.map((p) => p.id)));

	function added(player: Player) {
		if (!players) return;
		players = [...players.filter((p) => p.id !== player.id), player];
	}

	function submitTeam(event: SubmitEvent) {
		event.preventDefault();
		if (first.id === null || second.id === null) {
			failures = { teams: t.pickBoth };
			return;
		}
		if (first.id === second.id) {
			failures = { teams: t.samePlayer };
			return;
		}
		const pair: [string, string] = [first.id, second.id];
		const name = teamName.trim();
		void act('teams', async () => {
			await addTeam(id, name === '' ? { players: pair } : { players: pair, name });
			first = { name: '', id: null };
			second = { name: '', id: null };
			teamName = '';
			await refresh();
		});
	}

	function startRename(team: EventTeam) {
		renaming = team.id;
		renameText = team.custom_name ?? '';
		failures = {};
	}

	function submitRename(event: SubmitEvent, team: EventTeam) {
		event.preventDefault();
		const name = renameText.trim();
		void act('teams', async () => {
			await updateTeam(id, team.id, { name: name === '' ? null : name });
			renaming = null;
			await refresh();
		});
	}

	function removeTeam(team: EventTeam) {
		void act('teams', async () => {
			await deleteTeam(id, team.id);
			await refresh();
		});
	}

	// ---- courts ----------------------------------------------------------------------------

	let rotating = $state<string | null>(null);

	const matchById = (mid: string | null) => (mid === null ? undefined : matches.find((m) => m.id === mid));
	const courtName = (cid: string | null) => courts.find((c) => c.id === cid)?.name ?? t.notOnCourt;

	function confirmRotate(court: CourtOut) {
		void act('courts', async () => {
			await rotateCourt(id, court.id);
			rotating = null;
			await refresh();
		});
	}

	function nextMatch(court: CourtOut) {
		void act('courts', async () => {
			detail = await advanceCourt(id, court.id);
		});
	}

	function newCourt() {
		void act('courts', async () => {
			await addCourt(id);
			await refresh();
		});
	}

	// ---- matches ---------------------------------------------------------------------------

	let teamA = $state('');
	let teamB = $state('');
	let matchCourt = $state('');
	let matchLabel = $state('');
	let sheetFor = $state<string | null>(null);

	function submitMatch(event: SubmitEvent) {
		event.preventDefault();
		if (teamA === '' || teamB === '') {
			failures = { matches: t.pickTeam };
			return;
		}
		if (teamA === teamB) {
			failures = { matches: t.sameTeam };
			return;
		}
		const label = matchLabel.trim();
		void act('matches', async () => {
			await createEventMatch(id, {
				team_a: teamA,
				team_b: teamB,
				...(matchCourt === '' ? {} : { court_id: matchCourt }),
				...(label === '' ? {} : { label })
			});
			teamA = '';
			teamB = '';
			matchCourt = '';
			matchLabel = '';
			await refresh();
		});
	}

	function move(match: EventMatch, courtId: string) {
		void act('matches', async () => {
			await assignMatch(id, match.id, courtId === '' ? null : courtId);
			await refresh();
		});
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
		await postEventResult(id, match.id, { client_id: newClientId(), score });
		sheetFor = null;
		await refresh().catch(() => {
			// The stream brings the change too.
		});
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

	// ---- start and finish ------------------------------------------------------------------

	let finishing = $state(false);
	let order = $state<string[]>([]);
	let tied = $state<boolean[]>([]);

	const preview = $derived(placesFromOrder(groupsFromTies(order, tied)));
	const teamById = (tid: string) => teams.find((x) => x.id === tid);
	const nameOf = (tid: string) => {
		const tm = teamById(tid);
		return tm ? teamText(tm) : t.unknownTeam;
	};
	const finalPlaces = $derived([...(detail?.places ?? [])].sort((x, y) => x.place - y.place));

	function start() {
		void act('lifecycle', async () => {
			detail = await startEvent(id);
		});
	}

	function openFinish() {
		order = teams.map((x) => x.id);
		tied = order.slice(1).map(() => false);
		finishing = true;
		failures = {};
	}

	function shift(i: number, by: number) {
		order = moveItem(order, i, i + by);
	}

	function finish() {
		void act('lifecycle', async () => {
			detail = await finishEvent(id, preview);
			finishing = false;
		});
	}
</script>

<main class="acct">
	<a class="back" href="/events">{t.allEvents}</a>
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{:else if $session.status === 'signed_in' && detail && ev}
		<header class="head">
			<h1>{ev.name}</h1>
			<p class="meta">
				<span>{t.kinds[ev.kind]}</span>
				<span class="badge" class:off={status === 'finished'}>{t.status[status]}</span>
			</p>
			<p class="hint">{rulesText(ev.format)}</p>
		</header>

		{#if streamStatus === 'offline' || streamStatus === 'gone'}
			<div class="lost" role="status">
				<span>{t.streamLost}</span>
				<button class="secondary" type="button" onclick={() => location.reload()}>{t.reload}</button>
			</div>
		{/if}

		<!-- 1. Teams -->
		<section aria-labelledby="h-teams">
			<h2 id="h-teams">{t.teams}</h2>
			{#if teams.length === 0}
				<p class="hint">{t.noTeams}</p>
			{:else}
				<ul class="list" aria-labelledby="h-teams">
					{#each teams as team (team.id)}
						<li class="row">
							{#if renaming === team.id}
								<form class="inline" onsubmit={(e) => submitRename(e, team)}>
									<!-- svelte-ignore a11y_autofocus -->
									<input
										bind:value={renameText}
										aria-label={t.newTeamName(teamText(team))}
										aria-describedby="h-rename"
										placeholder={strings.teamName(team.players[0].name, team.players[1].name)}
										autocomplete="off"
										maxlength={MAX_NAME_LENGTH}
										autofocus
									/>
									<p class="hint" id="h-rename">{t.newTeamNameHint}</p>
									<div class="actions">
										<button class="primary" type="submit" disabled={busy}>{t.save}</button>
										<button class="secondary" type="button" onclick={() => (renaming = null)}>{t.cancel}</button>
									</div>
								</form>
							{:else}
								<span class="what">
									<strong>{teamText(team)}</strong>
									{#if team.custom_name !== null}
										<span class="hint">{strings.teamName(team.players[0].name, team.players[1].name)}</span>
									{/if}
								</span>
								{#if status !== 'finished'}
									<div class="actions">
										<button class="secondary" type="button" disabled={busy} aria-label={t.renameLabel(teamText(team))} onclick={() => startRename(team)}>
											{t.rename}
										</button>
										{#if status === 'setup'}
											<button class="danger" type="button" disabled={busy} aria-label={t.removeLabel(teamText(team))} onclick={() => removeTeam(team)}>
												{t.remove}
											</button>
										{/if}
									</div>
								{/if}
							{/if}
						</li>
					{/each}
				</ul>
			{/if}

			{#if status === 'setup'}
				<h3>{t.addTeam}</h3>
				{#if players}
					<form class="add-team" onsubmit={submitTeam}>
						<div class="slots">
							<div class="slot">
								<PlayerPicker
									bind:value={first.name}
									bind:playerId={first.id}
									{players}
									taken={[...inTeams, ...(second.id ? [second.id] : [])]}
									placeholder={t.teamPlayer(1)}
									label={t.teamPlayer(1)}
									onadd={added}
								/>
							</div>
							<div class="slot">
								<PlayerPicker
									bind:value={second.name}
									bind:playerId={second.id}
									{players}
									taken={[...inTeams, ...(first.id ? [first.id] : [])]}
									placeholder={t.teamPlayer(2)}
									label={t.teamPlayer(2)}
									onadd={added}
								/>
							</div>
						</div>
						{#if players.length === 0}<p class="hint">{t.noPlayers}</p>{/if}
						<label>
							{t.teamName}
							<input bind:value={teamName} autocomplete="off" maxlength={MAX_NAME_LENGTH} />
						</label>
						<button class="primary" type="submit" disabled={busy}>{t.addTeam}</button>
					</form>
				{:else if playersFailed}
					<p class="hint" role="status">{strings.players.listFailed}</p>
				{:else}
					<p class="hint">{strings.auth.loading}</p>
				{/if}
			{/if}
			{#if failures.teams}<p class="error" role="alert">{failures.teams}</p>{/if}
		</section>

		<!-- 2. Courts -->
		<section aria-labelledby="h-courts">
			<h2 id="h-courts">{t.courts}</h2>
			<div class="cards">
				{#each courts as court (court.id)}
					{@const links = courtLinks(origin, court)}
					{@const current = matchById(court.current)}
					<article class="card" aria-labelledby="court-{court.id}">
						<h3 id="court-{court.id}">{court.name}</h3>
						<p class="st">{courtStatusText(court, matches, teams)}</p>
						<div class="qrs">
							{#each [{ key: 'scoring', label: t.scoring, qr: t.scoringQr(court.name), link: links.scoring }, { key: 'viewing', label: t.viewing, qr: t.viewingQr(court.name), link: links.viewing }] as item (item.key)}
								{@const copyKey = `${court.id}-${item.key}`}
								<div class="qr">
									<figure>
										<QrCode text={item.link} label={item.qr} />
										<figcaption>{item.label}</figcaption>
									</figure>
									<button class="secondary small" type="button" aria-label={t.copyLabel(court.name, item.label)} onclick={() => copy(copyKey, item.link)}>
										{copied === copyKey ? t.copied : t.copy}
									</button>
								</div>
							{/each}
						</div>
						{#if status !== 'finished'}
							<div class="actions">
								{#if status === 'running' && current?.state === 'finished'}
									<button class="primary" type="button" disabled={busy} onclick={() => nextMatch(court)}>{t.nextMatch}</button>
								{/if}
								{#if rotating === court.id}
									<div class="confirm" role="group" aria-label={t.rotate}>
										<p>{t.rotateConfirm}</p>
										<div class="actions">
											<button class="danger" type="button" disabled={busy} onclick={() => confirmRotate(court)}>{t.confirm}</button>
											<button class="secondary" type="button" onclick={() => (rotating = null)}>{t.cancel}</button>
										</div>
									</div>
								{:else}
									<button class="secondary" type="button" disabled={busy} onclick={() => (rotating = court.id)}>{t.rotate}</button>
								{/if}
							</div>
						{/if}
					</article>
				{/each}

				<article class="card" aria-labelledby="court-whole">
					<h3 id="court-whole">{t.wholeEvent}</h3>
					<p class="st">{t.wholeEventHint}</p>
					<div class="qrs">
						<div class="qr">
							<figure>
								<QrCode text={eventLink(origin, detail.spectator_token)} label={t.wholeEventQr} />
								<figcaption>{t.viewing}</figcaption>
							</figure>
							<button
								class="secondary small"
								type="button"
								aria-label={t.copyLabel(t.wholeEventPlace, t.viewing)}
								onclick={() => copy('whole', eventLink(origin, detail?.spectator_token ?? ''))}
							>
								{copied === 'whole' ? t.copied : t.copy}
							</button>
						</div>
					</div>
				</article>
			</div>
			{#if copyFailed}<p class="hint" role="alert">{t.copyFailed}</p>{/if}
			{#if status !== 'finished'}
				<button class="secondary wide" type="button" disabled={busy} onclick={newCourt}>{t.addCourt}</button>
			{/if}
			{#if failures.courts}<p class="error" role="alert">{failures.courts}</p>{/if}
		</section>

		<!-- 3. Matches -->
		{#if status !== 'setup'}
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
											<select value={m.court_id ?? ''} disabled={busy} onchange={(e) => move(m, e.currentTarget.value)}>
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
										<button class="secondary" type="button" disabled={busy} aria-label={t.enterResultLabel(text)} onclick={() => (sheetFor = m.id)}>
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
				{#if failures.matches}<p class="error" role="alert">{failures.matches}</p>{/if}
			</section>
		{/if}

		<!-- 4. The kind's own panel -->
		<KindPanel event={detail} audience="coordinator" />

		<!-- 5. Start and finish -->
		<section aria-labelledby="h-lifecycle">
			{#if status === 'setup'}
				<h2 id="h-lifecycle" class="visually-hidden">{t.start}</h2>
				<p class="hint">{t.startHint}</p>
				<button class="primary wide" type="button" disabled={busy} onclick={start}>{t.start}</button>
			{:else if status === 'running'}
				<h2 id="h-lifecycle">{finishing ? t.finishTitle : t.finish}</h2>
				{#if finishing}
					<p class="hint">{t.finishHint}</p>
					<ol class="places">
						{#each order as tid, i (tid)}
							{@const place = preview.find((p) => p.team_id === tid)}
							{#if i > 0}
								<li class="tie">
									<label>
										<input type="checkbox" bind:checked={tied[i - 1]} aria-label={t.tiedLabel(nameOf(order[i - 1] ?? ''), nameOf(tid))} />
										<span aria-hidden="true">{t.tied}</span>
									</label>
								</li>
							{/if}
							<li class="row place-row">
								<span class="pos">{place ? t.place(place.place, place.place_to) : ''}</span>
								<span class="what"><strong>{nameOf(tid)}</strong></span>
								<div class="actions">
									<button class="secondary" type="button" disabled={i === 0} aria-label={t.moveUp(nameOf(tid))} onclick={() => shift(i, -1)}>{t.up}</button>
									<button class="secondary" type="button" disabled={i === order.length - 1} aria-label={t.moveDown(nameOf(tid))} onclick={() => shift(i, 1)}>{t.down}</button>
								</div>
							</li>
						{/each}
					</ol>
					<div class="actions">
						<button class="primary" type="button" disabled={busy} onclick={finish}>{busy ? t.finishing : t.finishConfirm}</button>
						<button class="secondary" type="button" onclick={() => (finishing = false)}>{t.cancel}</button>
					</div>
				{:else}
					<button class="primary wide" type="button" disabled={busy} onclick={openFinish}>{t.finish}</button>
				{/if}
			{:else}
				<h2 id="h-lifecycle">{t.finishTitle}</h2>
				<ol class="list final">
					{#each finalPlaces as p (p.team_id)}
						<li class="row">
							<span class="pos">{t.place(p.place, p.place_to)}</span>
							<span class="what"><strong>{nameOf(p.team_id)}</strong></span>
						</li>
					{/each}
				</ol>
			{/if}
			{#if failures.lifecycle}<p class="error" role="alert">{failures.lifecycle}</p>{/if}
		</section>
	{:else}
		<p class="hint">{t.loading}</p>
	{/if}
</main>

<style>
	.back {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
	}
	.head h1 {
		margin-bottom: 4px;
		overflow-wrap: anywhere;
	}
	.meta {
		display: flex;
		align-items: center;
		gap: 4px;
		margin: 0 0 4px;
	}
	.meta .badge {
		margin-left: 6px;
	}
	section {
		margin-top: 8px;
	}
	h3 {
		margin: 20px 0 8px;
		font-size: 16px;
		font-weight: 700;
	}
	.lost {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		margin-top: 12px;
		padding: 8px 12px;
		border: 1px solid var(--warn);
		border-radius: 12px;
		color: var(--warn);
	}
	.row .what {
		flex: 1 1 10rem;
	}
	.inline {
		flex: 1;
		min-width: 0;
		gap: 8px;
	}
	.inline .hint {
		margin: 0;
	}
	.slots {
		display: grid;
		gap: 8px;
	}
	.slot {
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		padding: 0 10px;
	}
	/* The picker's field sits borderless inside the slot, as in New match. */
	.slot :global(input) {
		border: 0;
		border-radius: 0;
		padding: 2px 0;
		background: transparent;
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
	.cards {
		display: grid;
		gap: 12px;
	}
	.card {
		border: 1px solid var(--line);
		border-radius: 16px;
		padding: 16px;
		background: var(--glass);
		min-width: 0;
	}
	.card h3 {
		margin: 0 0 2px;
		font-size: 18px;
		overflow-wrap: anywhere;
	}
	.card .st {
		font-size: 13px;
		color: var(--muted);
		margin-bottom: 12px;
		overflow-wrap: anywhere;
	}
	.qrs {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 14px;
		margin-bottom: 12px;
	}
	.qr {
		margin: 0;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 6px;
		min-width: 0;
		text-align: center;
		font-size: 13px;
		color: var(--muted);
	}
	.qr figure {
		margin: 0;
		width: 100%;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 4px;
	}
	.qr :global(.code) {
		width: 100%;
		max-width: 180px;
	}
	.small {
		width: 100%;
		padding: 0 8px !important;
		font-size: 14px !important;
	}
	.confirm {
		width: 100%;
	}
	.confirm p {
		margin: 0 0 8px;
		font-size: 14px;
	}
	.wide {
		width: 100%;
		margin-top: 12px;
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
	.places {
		list-style: none;
		margin: 0 0 12px;
		padding: 0;
		display: grid;
		gap: 4px;
	}
	.tie label {
		flex-direction: row;
		align-items: center;
		justify-content: center;
		gap: 8px;
		min-height: var(--tap);
		color: var(--ink);
		font-size: 14px;
		cursor: pointer;
	}
	.tie input {
		width: 22px;
		min-height: 22px;
		height: 22px;
		margin: 0;
	}
	.pos {
		flex: none;
		min-width: 4.5rem;
		font-weight: 750;
		font-feature-settings: 'tnum';
	}
	.place-row .what {
		flex: 1 1 6rem;
	}
	.final {
		margin: 0;
	}
	p.error {
		margin-top: 12px;
	}
</style>
