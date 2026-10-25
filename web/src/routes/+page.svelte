<script lang="ts">
	import { strings } from '#lib/strings.ts';
	import { listRecent, type RecentMatch } from '#lib/setup.ts';

	let recent = $state<RecentMatch[]>([]);
	$effect(() => {
		recent = listRecent(localStorage);
	});
</script>

<main>
	<h1>{strings.appName}</h1>
	<p class="intro">{strings.homeIntro}</p>
	<a class="primary" href="/new">{strings.newMatch}</a>

	<h2>{strings.recentMatches}</h2>
	{#if recent.length === 0}
		<p class="hint">{strings.noRecent}</p>
	{:else}
		<ul>
			{#each recent as match (match.scorer_token)}
				<li>
					<a href={`/s/${encodeURIComponent(match.scorer_token)}`}>
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
		padding: 32px 16px 48px;
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
