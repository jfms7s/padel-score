<!--
	What /j/{token} shows for a scheduled match: when, where and the rules, the four places (a
	name or Open), the line, and Join or Leave. The organiser also has Edit, Remove, Cancel and
	Start match. The page reads and runs the calls; this shows and asks.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { browserTimeZone, toLocalInputs } from '#lib/dates.ts';
	import { rulesText } from '#lib/events.ts';
	import {
		MATCH_SECS,
		START_WINDOW_SECS,
		matchActions,
		matchLink,
		matchWhen,
		scheduledChange
	} from '#lib/scheduled.ts';
	import { strings } from '#lib/strings.ts';
	import type { ScheduledChange, ScheduledView } from '#lib/types.ts';

	let {
		view,
		origin,
		signedIn,
		sessionLoading,
		working,
		error,
		note,
		onjoin,
		onleave,
		onsignin,
		onshare,
		onstart,
		oncancel,
		onremove,
		onedit
	}: {
		view: ScheduledView;
		origin: string;
		signedIn: boolean;
		/** The session is still being read: neither the sign-in button nor the signed-in action shows. */
		sessionLoading: boolean;
		working: boolean;
		error: string | null;
		/** A short confirmation, like "Link copied". */
		note: string | null;
		onjoin: () => void;
		onleave: () => void;
		onsignin: () => void;
		onshare: (url: string, title: string) => void;
		onstart: () => void;
		oncancel: () => void;
		onremove: (slot: number) => void;
		/** Resolves true when the change was saved. */
		onedit: (change: ScheduledChange) => Promise<boolean>;
	} = $props();

	const t = strings.scheduled;
	const zone = browserTimeZone();

	let now = $state(Math.floor(Date.now() / 1000));
	onMount(() => {
		const timer = setInterval(() => (now = Math.floor(Date.now() / 1000)), 30_000);
		return () => clearInterval(timer);
	});

	const me = $derived(view.me);
	const actions = $derived(matchActions(view, now));
	const live = $derived(view.state === 'open' || view.state === 'full');
	const owner = $derived(me?.owner === true);
	const filled = $derived(view.places.filter((p) => p.name !== null).length);
	const when = $derived(matchWhen(view));
	const link = $derived(matchLink(origin, view.join_token));
	const beforeWindow = $derived(now < view.starts_at - START_WINDOW_SECS);
	const inWindow = $derived(!beforeWindow && now < view.starts_at + MATCH_SECS);

	let confirmLeave = $state(false);
	let confirmCancel = $state(false);
	let removing = $state<number | null>(null);
	let editing = $state(false);
	let form = $state({ date: '', time: '', place: '' });

	function openEdit() {
		const at = toLocalInputs(view.starts_at);
		form = { date: at.date, time: at.time, place: view.place ?? '' };
		editing = true;
	}

	async function saveEdit(event: SubmitEvent) {
		event.preventDefault();
		const change = scheduledChange(view, form);
		if (change === null) {
			editing = false;
			return;
		}
		if (await onedit(change)) editing = false;
	}

	/** Where you stand in the line (0-based). Only your own spot carries an id, except for the organiser, who sees every one. */
	const mineOnLine = $derived(
		me?.in_line === true && !owner ? view.line.findIndex((p) => p.user_id !== null) : -1
	);

	// Focus goes where the closed confirm was: back on its button when dismissed, on the heading
	// when the action is sent (the card it sat in may be gone by the time the page is read again).
	let placesHeading: HTMLElement | undefined = $state();
	let pageHeading: HTMLElement | undefined = $state();
	let removeConfirmEl: HTMLElement | undefined = $state();
	let leaveConfirmEl: HTMLElement | undefined = $state();
	let cancelConfirmEl: HTMLElement | undefined = $state();
	let leaveButton: HTMLElement | undefined = $state();
	let cancelButton: HTMLElement | undefined = $state();
	let removeButtons = $state<Record<number, HTMLElement | undefined>>({});

	$effect(() => {
		if (removing !== null && removeConfirmEl) removeConfirmEl.focus();
	});
	$effect(() => {
		if (confirmLeave && leaveConfirmEl) leaveConfirmEl.focus();
	});
	$effect(() => {
		if (confirmCancel && cancelConfirmEl) cancelConfirmEl.focus();
	});

	function keepPlace() {
		const slot = removing;
		removing = null;
		queueMicrotask(() => (slot === null ? undefined : removeButtons[slot])?.focus());
	}
	function keepLeave() {
		confirmLeave = false;
		queueMicrotask(() => leaveButton?.focus());
	}
	function keepMatch() {
		confirmCancel = false;
		queueMicrotask(() => cancelButton?.focus());
	}

	const removeName = $derived(removing === null ? '' : (view.places.find((p) => p.slot === removing)?.name ?? ''));
</script>

<header class="head">
	<h1 tabindex="-1" bind:this={pageHeading}>{t.title}</h1>
	<p class="when">{when}</p>
	<p class="hint">{rulesText(view.format)}</p>
	<p class="hint">{t.by(view.owner_name)}</p>
</header>

{#if view.state === 'cancelled'}
	<p class="banner" role="status">{t.cancelledBanner}</p>
{:else if view.state === 'started'}
	<p class="banner" role="status">{t.startedBanner}</p>
	{#if view.match_spectator_token}
		<a class="watch" href="/v/{encodeURIComponent(view.match_spectator_token)}">{t.watch}</a>
	{/if}
{:else if view.state === 'past'}
	<p class="banner" role="status">{t.pastBanner}</p>
{/if}

{#if error}<p class="error" role="alert">{error}</p>{/if}
<div aria-live="polite">{#if note}<p class="ok">{note}</p>{/if}</div>

<section aria-labelledby="h-places">
	<h2 id="h-places" tabindex="-1" bind:this={placesHeading}>{t.placesTitle}</h2>
	<ul class="list" aria-label={t.placesListLabel}>
		{#each view.places as p (p.slot)}
			<li class="row">
				<span class="what">
					{#if p.name !== null}
						<strong>
							{p.slot + 1}. {p.name}
							{#if me?.slot === p.slot}<span class="badge">{t.you}</span>{/if}
						</strong>
					{:else}
						<span class="open">{p.slot + 1}. {t.openPlace}</span>
					{/if}
				</span>
				{#if owner && live && p.name !== null}
					<button
						class="secondary small"
						type="button"
						bind:this={removeButtons[p.slot]}
						disabled={working}
						aria-label={t.removeLabel(p.name)}
						onclick={() => (removing = p.slot)}
					>
						{t.remove}
					</button>
				{/if}
			</li>
		{/each}
	</ul>
	{#if live && filled < 4}<p class="hint">{t.short(4 - filled)}</p>{/if}
	{#if removing !== null}
		<div class="confirm" role="group" tabindex="-1" bind:this={removeConfirmEl} aria-label={t.removeConfirm(removeName)}>
			<p>{t.removeConfirm(removeName)}</p>
			<div class="actions">
				<button
					class="danger"
					type="button"
					disabled={working}
					onclick={() => {
						const slot = removing;
						removing = null;
						placesHeading?.focus();
						if (slot !== null) onremove(slot);
					}}
				>
					{t.removeYes}
				</button>
				<button class="secondary" type="button" onclick={keepPlace}>{t.keep}</button>
			</div>
		</div>
	{/if}
</section>

{#if view.line.length > 0}
	<section aria-labelledby="h-line">
		<h2 id="h-line">{t.lineTitle}</h2>
		<ul class="list" aria-label={t.lineTitle}>
			{#each view.line as p, i (p.slot)}
				<li class="row">
					<span class="what">
						<strong>
							{i + 1}. {p.name ?? ''}
							{#if mineOnLine === i}<span class="badge">{t.you}</span>{/if}
						</strong>
					</span>
				</li>
			{/each}
		</ul>
		<p class="hint">{t.lineHint}</p>
	</section>
{/if}

{#if me && (me.slot !== null || me.in_line)}
	<p class="status" role="status">
		{me.slot !== null ? t.youAreIn(me.slot + 1) : mineOnLine >= 0 ? t.youAreOnLine(mineOnLine + 1) : t.youAreOnLineAnon}
	</p>
{/if}

{#if sessionLoading}
	<!-- Neither action until we know who is looking. -->
{:else if actions.join}
	<section class="card" aria-label={t.joinCardLabel}>
		<button class="primary" type="button" disabled={working} onclick={onjoin}>
			{working ? t.joining : view.state === 'full' ? t.joinLine : t.join}
		</button>
		<p class="hint">{t.joinNote}</p>
	</section>
{:else if !signedIn && live}
	<section class="card" aria-label={t.joinCardLabel}>
		<button class="primary" type="button" onclick={onsignin}>{t.signInToJoin}</button>
	</section>
{/if}

{#if actions.leave}
	<section class="card" aria-label={t.leave}>
		{#if confirmLeave}
			<p tabindex="-1" bind:this={leaveConfirmEl}>{t.leaveConfirm}</p>
			<div class="actions">
				<button
					class="danger"
					type="button"
					disabled={working}
					onclick={() => {
						confirmLeave = false;
						placesHeading?.focus();
						onleave();
					}}
				>
					{working ? t.leaving : t.leave}
				</button>
				<button class="secondary" type="button" onclick={keepLeave}>{t.keep}</button>
			</div>
		{:else}
			<button class="danger" type="button" bind:this={leaveButton} disabled={working} onclick={() => (confirmLeave = true)}>{t.leave}</button>
		{/if}
	</section>
{/if}

{#if live}
	<section class="card" aria-labelledby="h-link">
		<h2 id="h-link">{t.linkTitle}</h2>
		<p class="hint">{t.linkHint}</p>
		<code class="url">{link}</code>
		<button class="secondary" type="button" onclick={() => onshare(link, t.shareTitle(when))}>{t.shareLink}</button>
	</section>
{/if}

{#if owner && live}
	<section class="card" aria-labelledby="h-owner">
		<h2 id="h-owner">{t.ownerTools}</h2>
		{#if actions.start}
			<button class="primary" type="button" disabled={working} onclick={onstart}>
				{working ? t.starting : t.start}
			</button>
		{:else if filled < 4}
			<p class="hint">{t.startNeedsFour}</p>
		{:else if beforeWindow}
			<p class="hint">{t.startOpens}</p>
		{:else if !inWindow}
			<p class="hint">{t.pastBanner}</p>
		{/if}

		{#if editing}
			<form onsubmit={saveEdit}>
				<h3>{t.editTitle}</h3>
				<div class="pair">
					<label>
						{t.date}
						<input type="date" required bind:value={form.date} />
					</label>
					<label>
						{t.time}
						<input type="time" required bind:value={form.time} />
					</label>
				</div>
				<p class="hint">{t.timeZoneNote(zone)}</p>
				<label>
					{t.place}
					<input bind:value={form.place} placeholder={t.placePlaceholder} autocomplete="off" maxlength="80" />
				</label>
				<div class="actions">
					<button class="primary" type="submit" disabled={working}>{working ? t.saving : t.save}</button>
					<button class="secondary" type="button" onclick={() => (editing = false)}>{t.cancelEdit}</button>
				</div>
			</form>
		{:else if actions.edit}
			<button class="secondary" type="button" disabled={working} onclick={openEdit}>{t.edit}</button>
		{/if}

		{#if confirmCancel}
			<p tabindex="-1" bind:this={cancelConfirmEl}>{t.cancelConfirm}</p>
			<div class="actions">
				<button
					class="danger"
					type="button"
					disabled={working}
					onclick={() => {
						confirmCancel = false;
						pageHeading?.focus();
						oncancel();
					}}
				>
					{t.cancelYes}
				</button>
				<button class="secondary" type="button" onclick={keepMatch}>{t.cancelNo}</button>
			</div>
		{:else}
			<button class="danger" type="button" bind:this={cancelButton} disabled={working} onclick={() => (confirmCancel = true)}>{t.cancelMatch}</button>
		{/if}
	</section>
{/if}

<style>
	.head h1 {
		margin-bottom: 4px;
		overflow-wrap: anywhere;
	}
	.head p {
		margin: 0 0 4px;
	}
	.when {
		font-weight: 650;
	}
	.banner {
		margin: 12px 0;
		padding: 12px;
		border: 1px solid var(--warn);
		border-radius: 12px;
		color: var(--warn);
	}
	.watch {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--ink);
		font-weight: 650;
	}
	.card {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 10px;
		margin: 16px 0;
		padding: 14px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
	}
	.card > * {
		margin: 0;
		max-width: 100%;
	}
	.card form {
		display: flex;
		flex-direction: column;
		gap: 10px;
		width: 100%;
	}
	label {
		display: flex;
		flex-direction: column;
		gap: 6px;
		min-width: 0;
		font-size: 13px;
		color: var(--muted);
	}
	.pair {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 10px;
	}
	h2 {
		font-size: 18px;
	}
	h3 {
		margin: 0;
		font-size: 16px;
		font-weight: 700;
	}
	.open {
		color: var(--muted);
		font-style: italic;
	}
	.small {
		flex: none;
	}
	.status {
		margin: 12px 0;
		font-weight: 650;
	}
	.confirm {
		margin-top: 8px;
		padding: 12px;
		border: 1px solid var(--warn);
		border-radius: 12px;
	}
	.confirm p {
		margin: 0 0 8px;
	}
	.url {
		display: block;
		width: 100%;
		padding: 10px 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
		font-size: 13px;
		overflow-wrap: anywhere;
	}
	.actions {
		align-items: center;
	}
	p.error {
		margin: 12px 0;
	}
</style>
