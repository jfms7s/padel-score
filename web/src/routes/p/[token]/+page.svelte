<script lang="ts">
	// A public profile: the display name, record, placings and redacted rows. No sign-in; a 404
	// (never public, left public, or rotated) reads "This profile isn't public."
	import { page } from '$app/state';
	import { ApiError } from '#lib/api.ts';
	import LinkedResultRow from '#lib/components/LinkedResultRow.svelte';
	import RecordPlacings from '#lib/components/RecordPlacings.svelte';
	import { getProfile, linkMessage } from '#lib/linking.ts';
	import { strings } from '#lib/strings.ts';
	import type { LinkedEntry, LinkedHistory } from '#lib/types.ts';

	const t = strings.linking.publicProfile;
	const token = $derived(page.params.token ?? '');

	let profile = $state.raw<LinkedHistory | null>(null);
	let items = $state.raw<LinkedEntry[]>([]);
	let next = $state<string | null>(null);
	let failure = $state<string | null>(null);
	let moreFailure = $state<string | null>(null);
	let busy = $state(false);

	$effect(() => {
		const wanted = token;
		let live = true;
		profile = null;
		items = [];
		next = null;
		failure = null;
		getProfile(wanted).then(
			(h) => {
				if (!live) return;
				profile = h;
				items = h.items;
				next = h.next;
			},
			(e: unknown) => {
				if (live) failure = e instanceof ApiError && e.status === 404 ? t.notPublic : linkMessage(e);
			}
		);
		return () => {
			live = false;
		};
	});

	async function more() {
		if (busy || !next) return;
		const wanted = token;
		busy = true;
		moreFailure = null;
		try {
			const h = await getProfile(wanted, next);
			if (wanted !== token) return;
			const seen = new Set(items.map((m) => m.match_id));
			items = [...items, ...h.items.filter((m) => !seen.has(m.match_id))];
			next = h.next;
		} catch (e) {
			if (wanted === token) moreFailure = linkMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head>
	<title>{profile ? t.title(profile.display_name) : strings.appName}</title>
	<meta name="robots" content="noindex" />
</svelte:head>

<main class="acct">
	{#if failure}
		<h1>{strings.appName}</h1>
		<p class="error" role="alert">{failure}</p>
	{:else if profile}
		<h1>{t.title(profile.display_name)}</h1>
		<RecordPlacings record={profile.record} placings={profile.placings} />
		<p class="hint note">{t.note}</p>
		{#if items.length > 0}
			<ul class="list" aria-label={strings.history.matches}>
				{#each items as entry (entry.match_id)}
					<LinkedResultRow {entry} />
				{/each}
			</ul>
			{#if moreFailure}<p class="error more-error" role="alert">{moreFailure}</p>{/if}
			{#if next}
				<button class="secondary more" type="button" disabled={busy} onclick={more}>{strings.history.loadMore}</button>
			{/if}
		{/if}
	{:else}
		<p class="hint">{strings.auth.loading}</p>
	{/if}
</main>

<style>
	.note {
		margin-top: 12px;
	}
	.more {
		width: 100%;
		margin-top: 12px;
	}
	.more-error {
		margin-top: 12px;
	}
</style>
