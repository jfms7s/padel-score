<script lang="ts">
	// My results: the combined record across every coordinator, the entries linked to this account,
	// my claims, and who may see my history.
	import { ApiError } from '#lib/api.ts';
	import LinkCode from '#lib/components/LinkCode.svelte';
	import LinkedResultRow from '#lib/components/LinkedResultRow.svelte';
	import RecordPlacings from '#lib/components/RecordPlacings.svelte';
	import {
		getPrivacy,
		linkMessage,
		myClaims,
		myLinks,
		myResults,
		rotateProfile,
		setPrivacy,
		unlinkMe,
		withdrawClaim
	} from '#lib/linking.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { LinkedEntry, LinkedHistory, MyClaim, MyLink, Privacy, Visibility } from '#lib/types.ts';

	const t = strings.linking.profile;
	const VISIBILITIES: Visibility[] = ['private', 'coordinators', 'public'];

	let results = $state.raw<LinkedHistory | null>(null);
	let items = $state.raw<LinkedEntry[]>([]);
	let next = $state<string | null>(null);
	let links = $state.raw<MyLink[]>([]);
	let claims = $state.raw<MyClaim[]>([]);
	let privacy = $state.raw<Privacy | null>(null);
	// The radio group's value: follows the stored setting, but the group writes it as soon as one is
	// picked, so a failed save has to put it back (see `choose`).
	let picked = $derived(privacy?.visibility);

	let ready = $state(false);
	let loadFailure = $state<string | null>(null);
	let busy = $state(false);
	let moreFailure = $state<string | null>(null);
	let actionFailure = $state<string | null>(null);
	let privacyFailure = $state<string | null>(null);
	let privacyDone = $state(false);
	let confirmingUnlink = $state<string | null>(null);
	let confirmingRotate = $state(false);
	let origin = $state('');

	const empty = $derived(links.length === 0 && claims.every((c) => c.status !== 'pending') && items.length === 0);
	const pending = $derived(claims.filter((c) => c.status === 'pending'));

	$effect(() => {
		origin = location.origin;
		let live = true;
		void (async () => {
			const s = await requireSignedIn();
			if (!live) return;
			if (s.status === 'unavailable') loadFailure = strings.networkError;
			if (s.status !== 'signed_in') return;
			const [r, l, c, p] = await Promise.allSettled([myResults(), myLinks(), myClaims(), getPrivacy()]);
			if (!live) return;
			if (r.status === 'fulfilled') {
				results = r.value;
				items = r.value.items;
				next = r.value.next;
			}
			if (l.status === 'fulfilled') links = l.value;
			if (c.status === 'fulfilled') claims = c.value;
			if (p.status === 'fulfilled') privacy = p.value;
			const failed = [r, l, c, p].find((x) => x.status === 'rejected');
			if (failed && failed.status === 'rejected') loadFailure = linkMessage(failed.reason);
			ready = true;
		})();
		return () => {
			live = false;
		};
	});

	/** Run a change, then read what it changed again. */
	async function act(run: () => Promise<void>) {
		if (busy) return;
		busy = true;
		actionFailure = null;
		try {
			await run();
		} catch (e) {
			actionFailure = linkMessage(e);
		} finally {
			busy = false;
		}
	}

	async function reload() {
		const [r, l, c] = await Promise.all([myResults(), myLinks(), myClaims()]);
		results = r;
		items = r.items;
		next = r.next;
		links = l;
		claims = c;
	}

	const unlinkEntry = (playerId: string) =>
		act(async () => {
			await unlinkMe(playerId);
			confirmingUnlink = null;
			await reload();
		});

	const withdraw = (id: string) =>
		act(async () => {
			await withdrawClaim(id);
			claims = await myClaims();
		});

	async function more() {
		if (busy || !next) return;
		busy = true;
		moreFailure = null;
		try {
			const h = await myResults(next);
			const seen = new Set(items.map((m) => m.match_id));
			items = [...items, ...h.items.filter((m) => !seen.has(m.match_id))];
			next = h.next;
		} catch (e) {
			moreFailure = linkMessage(e);
		} finally {
			busy = false;
		}
	}

	async function choose(v: Visibility) {
		if (busy || privacy?.visibility === v) return;
		busy = true;
		privacyFailure = null;
		privacyDone = false;
		try {
			privacy = await setPrivacy(v);
			confirmingRotate = false;
			privacyDone = true;
		} catch (e) {
			privacyFailure = linkMessage(e);
			// The group already shows the unsaved choice; snap it back to the stored one.
			picked = privacy?.visibility;
		} finally {
			busy = false;
		}
	}

	async function rotate() {
		if (busy) return;
		busy = true;
		privacyFailure = null;
		try {
			privacy = await rotateProfile();
			confirmingRotate = false;
		} catch (e) {
			privacyFailure = e instanceof ApiError ? linkMessage(e) : strings.networkError;
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct">
	<a class="back" href="/">{t.back}</a>
	<h1>{t.title}</h1>
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{/if}
	{#if $session.status === 'signed_in' && ready}
		{#if empty}
			<p class="hint">{t.empty}</p>
		{/if}

		{#if results}
			<RecordPlacings record={results.record} placings={results.placings} />
			{#if items.length > 0}
				<ul class="list matches" aria-label={t.title}>
					{#each items as entry (entry.match_id)}
						<LinkedResultRow {entry} />
					{/each}
				</ul>
				{#if moreFailure}<p class="error more-error" role="alert">{moreFailure}</p>{/if}
				{#if next}
					<button class="secondary more" type="button" disabled={busy} onclick={more}>{t.loadMore}</button>
				{/if}
			{/if}
		{/if}

		{#if actionFailure}<p class="error" role="alert">{actionFailure}</p>{/if}

		{#if links.length > 0}
			<h2>{t.linkedEntries}</h2>
			<ul class="list" aria-label={t.linkedEntries}>
				{#each links as link (link.player_id)}
					<li class="row">
						<span class="what">{t.linkedEntry(link.name, link.coordinator_name)}</span>
						{#if confirmingUnlink === link.player_id}
							<p class="confirm">{t.unlinkConfirm(link.name)}</p>
							<div class="actions">
								<button class="danger" type="button" disabled={busy} onclick={() => unlinkEntry(link.player_id)}
									>{strings.linking.unlinkYes}</button
								>
								<button class="secondary" type="button" onclick={() => (confirmingUnlink = null)}
									>{strings.linking.keep}</button
								>
							</div>
						{:else}
							<button
								class="secondary"
								type="button"
								disabled={busy}
								aria-label={t.unlinkLabel(link.name, link.coordinator_name)}
								onclick={() => (confirmingUnlink = link.player_id)}>{strings.linking.unlink}</button
							>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}

		{#if claims.length > 0}
			<h2>{t.claimsTitle}</h2>
			<ul class="list" aria-label={t.claimsTitle}>
				{#each claims as claim (claim.id)}
					<li class="row">
						<span class="what">
							<strong>{t.claimLine(claim.preview.name, claim.preview.coordinator_name)}</strong>
							<span class="hint">{t.claimStatus[claim.status]}</span>
						</span>
						{#if claim.status === 'pending'}
							<button
								class="secondary"
								type="button"
								disabled={busy}
								aria-label={t.withdrawLabel(claim.preview.name)}
								onclick={() => withdraw(claim.id)}>{t.withdraw}</button
							>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}

		{#if privacy}
			<h2>{t.privacyTitle}</h2>
			<fieldset class="privacy" disabled={busy}>
				<legend class="visually-hidden">{t.privacyTitle}</legend>
				{#each VISIBILITIES as v (v)}
					<label class="option">
						<input
							type="radio"
							name="visibility"
							value={v}
							bind:group={picked}
							onchange={() => choose(v)}
						/>
						<span class="what"
							><strong>{t.privacyOptions[v].label}</strong><span class="hint">{t.privacyOptions[v].hint}</span></span
						>
					</label>
				{/each}
			</fieldset>
			{#if privacyFailure}<p class="error" role="alert">{privacyFailure}</p>{/if}
			{#if privacyDone}<p class="ok" role="status">{t.privacySaved}</p>{/if}
			{#if privacy.visibility === 'public' && privacy.profile_url}
				<LinkCode url={origin + privacy.profile_url} title={t.profileLink} hint={t.profileLinkHint} />
				{#if confirmingRotate}
					<p class="confirm">{t.newLinkConfirm}</p>
					<div class="actions">
						<button class="danger" type="button" disabled={busy} onclick={rotate}>{t.newLinkYes}</button>
						<button class="secondary" type="button" onclick={() => (confirmingRotate = false)}>{t.keepLink}</button>
					</div>
				{:else}
					<button class="secondary rotate" type="button" disabled={busy} onclick={() => (confirmingRotate = true)}
						>{t.newLink}</button
					>
				{/if}
			{/if}
		{/if}
	{:else if !loadFailure}
		<p class="hint">{strings.auth.loading}</p>
	{/if}
</main>

<style>
	.back {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
	}
	.matches {
		margin-top: 12px;
	}
	.more,
	.rotate {
		width: 100%;
		margin-top: 12px;
	}
	.more-error {
		margin-top: 12px;
	}
	.confirm {
		flex: 1 1 100%;
		margin: 0;
	}
	.privacy {
		display: grid;
		gap: 8px;
		margin: 0;
		padding: 0;
		border: 0;
		min-width: 0;
	}
	.option {
		display: flex;
		flex-direction: row;
		align-items: center;
		gap: 12px;
		min-height: var(--tap);
		padding: 10px 14px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		font-size: 16px;
	}
	.option:has(input:checked) {
		border-color: var(--ink);
	}
	.option:has(input:focus-visible) {
		outline: 2px solid var(--ink);
		outline-offset: 2px;
	}
	.option input {
		flex: none;
		width: 22px;
		min-height: 22px;
		height: 22px;
		padding: 0;
		accent-color: var(--ink);
	}
</style>
