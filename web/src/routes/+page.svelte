<script lang="ts">
	import { authMessage, listMyMatches } from '#lib/auth.ts';
	import DeleteMatchSheet from '#lib/components/DeleteMatchSheet.svelte';
	import { dateText } from '#lib/dates.ts';
	import { homeEvents, listEvents } from '#lib/events.ts';
	import { claimBadgeCount } from '#lib/linking.ts';
	import { listPlayers } from '#lib/players.ts';
	import { session } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import { listRecent, removeRecent, scorerPath, type RecentMatch } from '#lib/setup.ts';
	import { safeStorage } from '#lib/storage.ts';
	import type { EventSummary, MyMatch } from '#lib/types.ts';

	let recent = $state<RecentMatch[]>([]);
	$effect(() => {
		recent = listRecent(safeStorage());
	});

	// Signed in, the server's list of live matches this user started, on any device.
	let mine = $state<MyMatch[] | null>(null);
	let mineFailure = $state<string | null>(null);
	const signedIn = $derived($session.status === 'signed_in');
	$effect(() => {
		if (!signedIn) return;
		let live = true;
		listMyMatches().then(
			(list) => {
				if (live) mine = list;
			},
			(e: unknown) => {
				if (live) mineFailure = authMessage(e);
			}
		);
		return () => {
			live = false;
		};
	});

	// Claims waiting on the coordinator, summed over the player list. Best effort: no badge on failure.
	let claimCount = $state(0);
	$effect(() => {
		if (!signedIn) return;
		let live = true;
		listPlayers({ quiet: true }).then(
			(list) => {
				if (live) claimCount = claimBadgeCount(list);
			},
			() => {}
		);
		return () => {
			live = false;
		};
	});

	// The wide layout's Events list; best effort, and hidden below 1100px.
	let events = $state<EventSummary[]>([]);
	$effect(() => {
		if (!signedIn) return;
		let live = true;
		listEvents().then(
			(list) => {
				if (live) events = homeEvents(list);
			},
			() => {}
		);
		return () => {
			live = false;
		};
	});

	// The match whose delete sheet is open. Once it is gone (or kept as finished) it leaves both
	// lists: the same match can be in each.
	let deleting = $state<{ label: string; scorerToken: string } | null>(null);
	function closeDelete(removed: boolean) {
		const token = deleting?.scorerToken;
		deleting = null;
		if (!removed || token === undefined) return;
		const storage = safeStorage();
		removeRecent(storage, token);
		recent = listRecent(storage);
		if (mine) mine = mine.filter((m) => m.scorer_token !== token);
	}

	/** Only a session gets the tokens; without them there is no page to open. */
	const openable = (m: MyMatch): m is MyMatch & { scorer_token: string } => typeof m.scorer_token === 'string';
</script>

{#snippet deleteButton(label: string, scorerToken: string)}
	<button
		class="delete"
		type="button"
		aria-label={strings.deleteMatch.button(label)}
		onclick={() => (deleting = { label, scorerToken })}
	>
		<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
			<path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
		</svg>
	</button>
{/snippet}

<main class:signed={signedIn} class:page-wide-desk={signedIn} class:page-narrow={!signedIn}>
	<div class="cols">
	<div class="col">
	<h1>{strings.appName}</h1>
	<p class="intro">{strings.homeIntro}</p>
	<a class="primary" href="/new">{strings.newMatch}</a>

	{#if signedIn}
		<nav class="links" aria-label={strings.yourPages}>
			<a href="/players"
				>{strings.players.title}{#if claimCount > 0}<span class="badge">{strings.linking.claimBadge(claimCount)}</span>{/if}</a
			>
			<a href="/history">{strings.history.link}</a>
			<a href="/events">{strings.events.link}</a>
			<a href="/boards/new">{strings.boards.newBoard}</a>
			<a class="wide" href="/profile">{strings.linking.profile.link}</a>
		</nav>
		<h2>{strings.myMatches}</h2>
		{#if mineFailure}
			<p class="hint" role="alert">{mineFailure}</p>
		{:else if mine === null}
			<p class="hint">{strings.auth.loading}</p>
		{:else if mine.filter(openable).length === 0}
			<p class="hint">{strings.noMyMatches}</p>
		{:else}
			<ul aria-label={strings.myMatches}>
				{#each mine.filter(openable) as match (match.id)}
					<li>
						<a href={scorerPath(match.scorer_token, match.spectator_token)}>
							<span class="label">{match.label}</span>
							<time datetime={new Date(match.last_activity_at * 1000).toISOString()}>
								{dateText(match.last_activity_at)}
							</time>
						</a>
						{#if !match.context}
							{@render deleteButton(match.label, match.scorer_token)}
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	{/if}

	</div>
	<div class="col">
	{#if signedIn}
		<section class="events">
			<h2>{strings.nav.events}</h2>
			{#if events.length === 0}
				<p class="hint">{strings.events.none}</p>
			{:else}
				<ul aria-label={strings.nav.events}>
					{#each events as ev (ev.id)}
						<li>
							<a href="/events/{encodeURIComponent(ev.id)}">
								<span class="label">{ev.name}</span>
								<span class="chip" class:run={ev.status === 'running'}>{strings.events.status[ev.status]}</span>
							</a>
						</li>
					{/each}
				</ul>
			{/if}
		</section>
	{/if}
	<h2>{strings.recentMatches}</h2>
	{#if recent.length === 0}
		<p class="hint">{strings.noRecent}</p>
	{:else}
		<ul>
			{#each recent as match (match.scorer_token)}
				<li>
					<a href={scorerPath(match.scorer_token, match.spectator_token)}>
						<span class="label">{match.label}</span>
						<time datetime={new Date(match.created_at).toISOString()}>
							{dateText(match.created_at / 1000)}
						</time>
					</a>
					{@render deleteButton(match.label, match.scorer_token)}
				</li>
			{/each}
		</ul>
	{/if}
	</div>
	</div>
</main>

{#if deleting}
	<DeleteMatchSheet label={deleting.label} scorerToken={deleting.scorerToken} onclose={closeDelete} />
{/if}

<style>
	main {
		padding-top: 16px;
		padding-bottom: 48px;
	}
	h1 {
		margin: 0 0 8px;
		font-size: 40px;
		font-weight: 800;
		font-variation-settings: 'wdth' 75;
		line-height: 1;
	}
	h2 {
		margin: 40px 0 12px;
		font-size: 18px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	.intro,
	.hint {
		color: var(--muted);
		margin: 0 0 24px;
	}
	.hint {
		margin: 0;
	}
	.primary {
		display: flex;
		align-items: center;
		justify-content: center;
		min-height: var(--tap);
		border-radius: 14px;
		background: var(--ink);
		color: var(--glass);
		font-size: 17px;
		font-weight: 650;
		text-decoration: none;
	}
	.links {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: 8px;
		margin-top: 8px;
	}
	.links a {
		display: flex;
		align-items: center;
		justify-content: center;
		min-height: var(--tap);
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--panel);
		color: var(--ink);
		font-size: 16px;
		font-weight: 650;
		text-decoration: none;
	}
	.links .wide {
		grid-column: 1 / -1;
	}
	.chip {
		flex: none;
		padding: 0 8px;
		border: 1px solid var(--line);
		border-radius: 999px;
		font-size: 12px;
		font-weight: 650;
		line-height: 22px;
	}
	.chip.run {
		background: var(--a-soft);
	}
	.events {
		display: none;
	}
	@media (min-width: 1100px) {
		main.signed .cols {
			display: grid;
			grid-template-columns: repeat(2, minmax(0, 1fr));
			gap: 0 48px;
			align-items: start;
		}
		main.signed .links {
			display: none;
		}
		main.signed .events {
			display: block;
		}
		main.signed .events h2 {
			margin-top: 0;
		}
	}
	.badge {
		margin-left: 8px;
		padding: 0 8px;
		border-radius: 999px;
		background: var(--a-soft);
		font-size: 12px;
		font-weight: 650;
		line-height: 22px;
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 8px;
	}
	li {
		display: flex;
		gap: 8px;
	}
	li a {
		flex: 1;
		min-width: 0;
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 12px;
		min-height: var(--tap);
		padding: 8px 14px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		text-decoration: none;
	}
	.delete {
		flex: none;
		display: flex;
		align-items: center;
		justify-content: center;
		width: var(--tap);
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--muted);
	}
	.label {
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	time {
		color: var(--muted);
		font-size: 13px;
		flex: none;
	}
</style>
