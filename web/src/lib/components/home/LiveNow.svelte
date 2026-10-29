<script lang="ts">
	// The matches being scored: signed in, the account's live matches; offline, this device's own.
	import type { LiveRow } from '#lib/home.ts';
	import { strings } from '#lib/strings.ts';

	let {
		fresh,
		older,
		offline,
		ondelete
	}: { fresh: LiveRow[]; older: LiveRow[]; offline: boolean; ondelete: (row: LiveRow) => void } = $props();

	const t = strings.home;
	let showOlder = $state(false);
	const rows = $derived(showOlder ? [...fresh, ...older] : fresh);
</script>

<section aria-labelledby="live">
	<h2 id="live">{t.live}</h2>
	{#if offline}<p class="hint">{t.offlineHint}</p>{/if}
	{#if rows.length === 0}
		<p class="hint">{offline ? strings.noRecent : strings.noMyMatches}</p>
	{:else}
		<ul>
			{#each rows as row (row.scorerToken)}
				<li>
					<a class="resume" href={row.href}>
						<span class="what">
							<span class="label">{row.label}</span>
							<span class="when">{row.when}</span>
						</span>
						{#if row.score !== null}<span class="score">{row.score}</span>{/if}
						<span class="go">{t.resume}</span>
					</a>
					{#if row.deletable}
						<button
							class="delete"
							type="button"
							aria-label={strings.deleteMatch.button(row.label)}
							onclick={() => ondelete(row)}
						>
							<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
								<path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
							</svg>
						</button>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
	{#if older.length > 0 && !showOlder}
		<button class="older" type="button" onclick={() => (showOlder = true)}>{t.older(older.length)}</button>
	{/if}
</section>

<style>
	h2 {
		margin: 24px 0 8px;
		font-size: 18px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	.hint {
		margin: 0 0 8px;
		color: var(--muted);
	}
	ul {
		list-style: none;
		margin: 0 0 8px;
		padding: 0;
		display: grid;
		gap: 8px;
	}
	li {
		display: flex;
		gap: 8px;
	}
	.resume {
		flex: 1;
		min-width: 0;
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 12px;
		min-height: var(--tap);
		padding: 8px 14px;
		border: 2px solid var(--ball);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		text-decoration: none;
	}
	.what {
		display: grid;
		min-width: 0;
	}
	.label {
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	.when {
		font-size: 13px;
		color: var(--muted);
	}
	.score {
		flex: none;
		margin-left: auto;
		font-size: 20px;
		font-weight: 800;
		font-variation-settings: 'wdth' 75;
		font-variant-numeric: tabular-nums;
	}
	.go {
		flex: none;
		padding: 0 12px;
		border-radius: 999px;
		background: var(--ball);
		color: var(--on-ball);
		font-weight: 700;
		line-height: 32px;
	}
	.delete {
		flex: none;
		display: flex;
		align-items: center;
		justify-content: center;
		width: var(--tap);
		min-height: var(--tap);
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--muted);
	}
	.older {
		display: flex;
		align-items: center;
		min-height: var(--tap);
		padding: 0 4px;
		border: 0;
		background: none;
		color: var(--muted);
		font-size: 15px;
		text-align: left;
	}
</style>
