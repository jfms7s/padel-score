<!--
	What /j/{token} shows for an event: when and where, the sign-up (alone, with a partner, or
	waiting on a partner link), the people in, and an invitation card first when the page was
	opened from a partner link. The page reads and runs the calls; this shows and asks.
-->
<script lang="ts">
	import { rulesText } from '#lib/events.ts';
	import { myEntryText, partnerLink, scheduleText } from '#lib/signups.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventJoinView, JoinAs, PartnerInvite, SignupTeam } from '#lib/types.ts';

	let {
		view,
		token,
		origin,
		signedIn,
		working,
		error,
		note,
		invite,
		inviteGone,
		inviteFailed,
		onretry,
		onjoin,
		onleave,
		onsignin,
		onshare,
		onaccept,
		ondecline
	}: {
		view: EventJoinView;
		token: string;
		origin: string;
		signedIn: boolean;
		working: boolean;
		error: string | null;
		/** A short confirmation, like "Link copied". */
		note: string | null;
		/** Who invites you, when the page was opened from a partner link. */
		invite: PartnerInvite | null;
		/** The partner link in the address does not work (any more). */
		inviteGone: boolean;
		/** The invitation could not be read (not a missing link): nothing is shown to join alone. */
		inviteFailed: boolean;
		onretry: () => void;
		onjoin: (as: JoinAs) => void;
		onleave: () => void;
		onsignin: () => void;
		onshare: (url: string, title: string) => void;
		onaccept: () => void;
		ondecline: () => void;
	} = $props();

	const t = strings.signups;
	const ev = $derived(view.event);
	const me = $derived(view.me);
	const cancelled = $derived(ev.status === 'cancelled');
	const started = $derived(ev.status === 'running' || ev.status === 'finished');
	const open = $derived(!ev.closed && !cancelled && !started);
	const when = $derived(scheduleText(ev.schedule));

	let withPartner = $state(false);
	let partnerName = $state('');
	let confirmLeave = $state(false);

	function continueWithPartner(event: SubmitEvent) {
		event.preventDefault();
		const name = partnerName.trim();
		onjoin(name === '' ? { as: 'partner' } : { as: 'partner', partner_name: name });
	}

	const names = (e: SignupTeam) => e.players.map((p) => p.name).join(' / ');
	const anyone = $derived(view.list.teams.length + view.list.line.length + view.list.solos.length > 0);
</script>

<header class="head">
	<h1>{ev.name}</h1>
	{#if when !== ''}<p class="when">{when}</p>{/if}
	<p class="hint">{strings.events.kinds[ev.kind]} · {rulesText(ev.format)}</p>
	<p class="hint">{t.by(ev.organiser_name)}</p>
</header>

{#if cancelled}
	<p class="banner" role="status">{t.cancelledBanner}</p>
{:else if started}
	<p class="banner" role="status">{t.startedBanner}</p>
{:else if ev.closed}
	<p class="banner" role="status">{t.closedBanner}</p>
{/if}

{#if inviteGone}
	<p class="error" role="alert">{t.inviteGone}</p>
{:else if inviteFailed}
	<p class="error" role="alert">{t.inviteFailed}</p>
	<button class="secondary" type="button" onclick={onretry}>{t.retry}</button>
{:else if invite}
	<section class="card" aria-labelledby="h-invite">
		<h2 id="h-invite">{t.inviteTitle(invite.inviter_name, invite.event_name)}</h2>
		<p class="hint">{t.inviteHint}</p>
		{#if signedIn}
			<div class="actions">
				<button class="primary" type="button" disabled={working || !open} onclick={onaccept}>{t.accept}</button>
				<button class="secondary" type="button" disabled={working} onclick={ondecline}>{t.decline}</button>
			</div>
		{:else}
			<button class="primary" type="button" onclick={onsignin}>{t.signInToAnswer}</button>
		{/if}
	</section>
{/if}

{#if error}<p class="error" role="alert">{error}</p>{/if}
<div aria-live="polite">{#if note}<p class="ok">{note}</p>{/if}</div>

{#if me}
	<section class="card" aria-labelledby="h-me">
		<h2 id="h-me">{myEntryText(me)}</h2>
		{#if me.state === 'pending'}
			{@const link = partnerLink(origin, token, me.partner_token)}
			<h3>{t.partnerLinkTitle}</h3>
			<p class="hint">{t.partnerLinkHint}</p>
			<code class="url">{link}</code>
			<button class="secondary" type="button" onclick={() => onshare(link, t.sharePartnerTitle(ev.name))}>
				{t.shareLinkPartner}
			</button>
		{/if}
		{#if confirmLeave}
			<p>{t.leaveConfirm}</p>
			<div class="actions">
				<button class="danger" type="button" disabled={working} onclick={onleave}>{working ? t.leaving : t.leave}</button>
				<button class="secondary" type="button" onclick={() => (confirmLeave = false)}>{t.keepEvent}</button>
			</div>
		{:else if !cancelled && !started}
			<button class="danger" type="button" disabled={working} onclick={() => (confirmLeave = true)}>{t.leave}</button>
		{/if}
	</section>
{:else if open && !inviteGone && !inviteFailed && !invite}
	<section class="card" aria-label={t.signupsLabel}>
		{#if !signedIn}
			<button class="primary" type="button" onclick={onsignin}>{t.signInToJoin}</button>
		{:else if withPartner}
			<form onsubmit={continueWithPartner}>
				<label>
					{t.partnerNameLabel}
					<input bind:value={partnerName} placeholder={t.partnerNamePlaceholder} autocomplete="off" maxlength="40" />
				</label>
				<p class="hint">{t.partnerNameHint}</p>
				<div class="actions">
					<button class="primary" type="submit" disabled={working}>{working ? t.joining : t.continueWithPartner}</button>
					<button class="secondary" type="button" onclick={() => (withPartner = false)}>{strings.events.cancel}</button>
				</div>
			</form>
		{:else}
			<div class="actions">
				<button class="primary" type="button" disabled={working} onclick={() => onjoin({ as: 'solo' })}>
					{working ? t.joining : t.joinAlone}
				</button>
				<button class="secondary" type="button" disabled={working} onclick={() => (withPartner = true)}>
					{t.joinWithPartner}
				</button>
			</div>
		{/if}
		<p class="hint">{t.joinNote}</p>
	</section>
{/if}

<section aria-labelledby="h-list">
	<h2 id="h-list">{t.listTitle}</h2>
	{#if !anyone}
		<p class="hint">{t.none}</p>
	{/if}
	{#each [[t.groups.teams, view.list.teams, true, view.list.hand_added], [t.groups.line, view.list.line, true, 0], [t.groups.solos, view.list.solos, false, 0]] as const as [title, entries, numbered, before] (title)}
		{#if entries.length > 0}
			<h3>{title}</h3>
			<ul class="list" aria-label={title}>
				{#each entries as entry, i (entry.id)}
					<li class="row">
						<span class="what"><strong>{numbered ? `${before + i + 1}. ` : ''}{names(entry)}</strong></span>
					</li>
				{/each}
			</ul>
		{/if}
	{/each}
</section>

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
		width: 100%;
	}
	h2 {
		font-size: 18px;
	}
	h3 {
		margin: 16px 0 8px;
		font-size: 16px;
		font-weight: 700;
	}
	.card h3 {
		margin: 4px 0 0;
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
