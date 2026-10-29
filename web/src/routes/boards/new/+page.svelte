<script lang="ts">
	// A new game board: a title, an optional place and the first of its seven days.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { boardMessage, createBoard } from '#lib/boards.ts';
	import { addDays, browserTimeZone, localDate, localMidnight, nextMonday } from '#lib/dates.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { MAX_NAME_LENGTH } from '#lib/setup.ts';
	import { strings } from '#lib/strings.ts';

	const t = strings.boards;
	const MAX_PLACE_LENGTH = 80;

	const zone = browserTimeZone();
	let title = $state('');
	let place = $state('');
	let firstDay = $state(nextMonday(localDate(Date.now() / 1000, zone)));
	let allowGuests = $state(false);
	let busy = $state(false);
	let failure = $state<string | null>(null);

	onMount(() => {
		void requireSignedIn();
	});

	async function create(event: SubmitEvent) {
		event.preventDefault();
		const name = title.trim();
		if (busy || name === '' || firstDay === '') return;
		busy = true;
		failure = null;
		try {
			const view = await createBoard({
				title: name,
				place: place.trim() === '' ? null : place.trim(),
				allow_guests: allowGuests,
				time_zone: zone,
				starts_at: localMidnight(firstDay, zone),
				ends_at: localMidnight(addDays(firstDay, 7), zone)
			});
			await goto(`/b/${encodeURIComponent(view.board.token)}`);
		} catch (e) {
			failure = boardMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct page-narrow">
	<a class="back" href="/">{strings.appName}</a>
	<h1>{t.newBoard}</h1>
	{#if $session.status === 'signed_in'}
		<p class="hint">{t.newIntro}</p>
		<form onsubmit={create}>
			<label>
				{t.title}
				<input bind:value={title} placeholder={t.titlePlaceholder} autocomplete="off" maxlength={MAX_NAME_LENGTH} required />
			</label>
			<label>
				{t.place}
				<input bind:value={place} placeholder={t.placePlaceholder} autocomplete="off" maxlength={MAX_PLACE_LENGTH} />
			</label>
			<label>
				{t.firstDay}
				<input type="date" bind:value={firstDay} required />
				<span class="hint">{t.firstDayHint}</span>
			</label>
			<label class="check">
				<input type="checkbox" bind:checked={allowGuests} />
				<span>{t.allowGuests}</span>
			</label>
			<p class="hint">{t.allowGuestsNote}</p>
			{#if failure}<p class="error" role="alert">{failure}</p>{/if}
			<button class="primary" type="submit" disabled={busy}>{busy ? t.creating : t.create}</button>
		</form>
	{:else if $session.status === 'unavailable'}
		<p class="error" role="alert">{strings.networkError}</p>
	{:else}
		<p class="hint">{strings.auth.loading}</p>
	{/if}
</main>

<style>
	.check {
		flex-direction: row;
		align-items: center;
		gap: 10px;
		min-height: var(--tap);
		color: var(--ink);
		font-size: 16px;
	}
	.check input {
		width: 24px;
		min-height: 24px;
		height: 24px;
		flex: none;
	}
	.back {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
	}
</style>
