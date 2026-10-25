<script lang="ts">
	import { authMessage, listMyMatches } from '#lib/auth.ts';
	import { session } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import { listRecent, scorerPath, type RecentMatch } from '#lib/setup.ts';
	import { safeStorage } from '#lib/storage.ts';
	import type { MyMatch } from '#lib/types.ts';

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

	/** Only a session gets the tokens; without them there is no page to open. */
	const openable = (m: MyMatch): m is MyMatch & { scorer_token: string } => typeof m.scorer_token === 'string';
</script>

<main>
	<h1>{strings.appName}</h1>
	<p class="intro">{strings.homeIntro}</p>
	<a class="primary" href="/new">{strings.newMatch}</a>

	{#if signedIn}
		<nav class="links" aria-label={strings.yourPages}>
			<a href="/players">{strings.players.title}</a>
			<a href="/history">{strings.history.link}</a>
			<a href="/events">{strings.events.link}</a>
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
								{new Date(match.last_activity_at * 1000).toLocaleDateString()}
							</time>
						</a>
					</li>
				{/each}
			</ul>
		{/if}
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
							{new Date(match.created_at).toLocaleDateString()}
						</time>
					</a>
				</li>
			{/each}
		</ul>
	{/if}
</main>

<style>
	main {
		max-width: 28rem;
		margin: 0 auto;
		padding: 16px 16px 48px;
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
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 8px;
	}
	li a {
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
