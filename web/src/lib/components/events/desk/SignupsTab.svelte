<!--
	The desk's Sign-ups tab: the date, place and sign-up settings (in setup), the join link to
	share, the people signed up (in the event, on the waiting list, waiting for a partner, or
	waiting for a partner to accept) with remove, split and pair, Take teams, and Cancel event.
	The page runs the calls and refreshes; this tab shows and asks.
-->
<script lang="ts">
	import ScheduleFields from '#lib/components/events/ScheduleFields.svelte';
	import { browserTimeZone } from '#lib/dates.ts';
	import {
		draftFromSchedule,
		joinLink,
		scheduleInput,
		type ScheduleDraft
	} from '#lib/signups.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventDetail, ScheduleInput, SignupList, SignupTeam, TakeReport } from '#lib/types.ts';

	let {
		detail,
		list,
		origin,
		legs,
		busy,
		failure,
		report,
		copied,
		copyFailed,
		onsave,
		onremove,
		onsplit,
		onpair,
		ontake,
		oncancel,
		onshare,
		oncopy
	}: {
		detail: EventDetail;
		/** Null until read. */
		list: SignupList | null;
		origin: string;
		legs: number;
		busy: boolean;
		failure: string | undefined;
		/** What Take teams just did. */
		report: TakeReport | null;
		copied: string | null;
		copyFailed: string | null;
		onsave: (input: ScheduleInput, done: () => void) => void;
		onremove: (entry: SignupTeam) => void;
		onsplit: (entry: SignupTeam) => void;
		onpair: (a: SignupTeam, b: SignupTeam) => void;
		ontake: () => void;
		oncancel: () => void;
		onshare: (link: string) => void;
		oncopy: (link: string) => void;
	} = $props();

	const t = strings.signups;
	const ev = $derived(detail.event);
	const schedule = $derived(ev.schedule);
	const setup = $derived(ev.status === 'setup');
	const cancelled = $derived(ev.status === 'cancelled');
	const link = $derived(detail.join_token === null ? null : joinLink(origin, detail.join_token));
	const closed = $derived(setup && schedule.signups !== 'off' && (schedule.deadline_at ?? Infinity) <= Math.floor(Date.now() / 1000));

	// ---- the schedule form ---------------------------------------------------------------

	let draft = $state<ScheduleDraft>(draftFromSchedule(null));
	let draftFor = '';
	let formError = $state<string | null>(null);
	let savedNote = $state(false);

	// Fill the form from the saved schedule when it changes (the first read, a save). A stream
	// message that leaves the schedule alone does not touch what the organiser is typing.
	$effect(() => {
		const key = JSON.stringify(schedule);
		if (key === draftFor) return;
		draftFor = key;
		draft = draftFromSchedule(schedule);
	});

	function save(event: SubmitEvent) {
		event.preventDefault();
		savedNote = false;
		const result = scheduleInput(draft, schedule, browserTimeZone());
		if ('error' in result) {
			formError = result.error === 'max_teams' ? strings.signups.fields.max_teams : t[result.error];
			return;
		}
		formError = null;
		if (result.ok === undefined) return;
		onsave(result.ok, () => (savedNote = true));
	}

	// ---- pairing -------------------------------------------------------------------------

	let picked = $state<string[]>([]);
	const solos = $derived(list?.solos ?? []);
	// A solo that went away (paired, removed) drops out of the pick.
	const pickedSolos = $derived(solos.filter((s) => picked.includes(s.id)));

	function togglePick(entry: SignupTeam) {
		picked = picked.includes(entry.id) ? picked.filter((id) => id !== entry.id) : [...picked, entry.id].slice(-2);
	}

	function pair() {
		const [a, b] = pickedSolos;
		if (a && b) {
			picked = [];
			onpair(a, b);
		}
	}

	// ---- cancel --------------------------------------------------------------------------

	let confirming = $state(false);
	let confirmEl: HTMLElement | undefined = $state();
	let cancelButton: HTMLElement | undefined = $state();

	$effect(() => {
		if (confirming && confirmEl) confirmEl.focus();
	});

	function closeConfirm() {
		confirming = false;
		queueMicrotask(() => cancelButton?.focus());
	}

	const names = (e: SignupTeam) => e.players.map((p) => p.name).join(' / ');
	const complete = $derived((list?.teams.length ?? 0) + (list?.line.length ?? 0));
</script>

<section aria-labelledby="h-signups">
	<h2 id="h-signups">{t.tabTitle}</h2>

	{#if cancelled}
		<p class="note" role="status">{t.cancelledNote}</p>
	{:else if closed}
		<p class="note" role="status">{t.closedNote}</p>
	{/if}

	<div class="layout">
		<div class="main">
			{#if link && !cancelled}
				<div class="link">
					<h3>{t.linkTitle}</h3>
					<p class="hint">{t.linkHint}</p>
					<code class="url">{link}</code>
					<div class="actions">
						<button class="secondary" type="button" onclick={() => oncopy(link)}>
							{copied === 'join' ? strings.events.copied : t.copyLink}
						</button>
						<button class="secondary" type="button" onclick={() => onshare(link)}>{t.shareLink}</button>
					</div>
					{#if copyFailed === 'join'}<p class="hint" role="alert">{strings.events.copyFailed}</p>{/if}
				</div>
			{:else if setup && schedule.starts_at === null && schedule.signups === 'off'}
				<p class="hint">{t.undated}</p>
			{/if}

			{#if list === null}
				{#if schedule.signups !== 'off' || cancelled}<p class="hint">{strings.auth.loading}</p>{/if}
			{:else}
				<p class="hint">
					{t.summary(list.teams.length + list.hand_added, schedule.max_teams)}
					{#if list.hand_added > 0}· {t.handAdded(list.hand_added)}{/if}
				</p>
				{@render group(t.groups.teams, list.teams, 'teams')}
				{@render group(t.groups.line, list.line, 'line')}
				{@render group(t.groups.solos, list.solos, 'solos')}
				{@render group(t.groups.pending, list.pending, 'pending')}
				{#if complete + solos.length + list.pending.length === 0 && schedule.signups !== 'off'}
					<p class="hint">{t.none}</p>
				{/if}
				{#if setup && solos.length >= 2}
					<div class="pairing">
						<p class="hint" id="h-pair">{t.pairHint} {t.pairPicked(pickedSolos.length)}</p>
						<button class="secondary" type="button" disabled={busy || pickedSolos.length !== 2} aria-describedby="h-pair" onclick={pair}>
							{t.pair}
						</button>
					</div>
				{/if}
			{/if}

			{#if report}
				<p class="ok" role="status">{t.takeReport(report.taken, report.skipped.length, report.left_on_line)}</p>
			{/if}
			{#if setup && schedule.signups !== 'off'}
				<div class="take">
					<button class="primary" type="button" disabled={busy} onclick={ontake}>{busy ? t.taking : t.takeTeams}</button>
					<p class="hint">{t.takeHint}</p>
				</div>
			{/if}
		</div>

		{#if setup}
			<div class="side">
				<form onsubmit={save}>
					<ScheduleFields bind:draft kind={ev.kind} {legs} prefix="sched" />
					{#if formError}<p class="error" role="alert">{formError}</p>{/if}
					<div class="actions">
						<button class="primary" type="submit" disabled={busy}>{t.saveSchedule}</button>
						{#if savedNote}<span class="hint" role="status">{t.saved}</span>{/if}
					</div>
				</form>

				<div class="danger-zone">
					{#if confirming}
						<div class="confirm" role="alertdialog" tabindex="-1" bind:this={confirmEl} aria-label={t.cancelEvent}>
							<p>{t.cancelConfirm}</p>
							<div class="actions">
								<button class="danger" type="button" disabled={busy} onclick={oncancel}>{busy ? t.cancelling : t.cancelEvent}</button>
								<button class="secondary" type="button" onclick={closeConfirm}>{t.keepEvent}</button>
							</div>
						</div>
					{:else}
						<button class="danger" type="button" disabled={busy} bind:this={cancelButton} onclick={() => (confirming = true)}>
							{t.cancelEvent}
						</button>
					{/if}
				</div>
			</div>
		{/if}
	</div>
	{#if failure}<p class="error" role="alert">{failure}</p>{/if}
</section>

{#snippet group(title: string, entries: SignupTeam[], which: 'teams' | 'line' | 'solos' | 'pending')}
	{#if entries.length > 0}
		<h3>{title}</h3>
		<ul class="list" aria-label={title}>
			{#each entries as entry, i (entry.id)}
				<li class="row">
					<span class="what">
						<strong>{which === 'teams' || which === 'line' ? `${i + 1}. ` : ''}{names(entry)}</strong>
						{#if entry.auto_paired}<span class="hint">{t.autoPaired}</span>{/if}
						{#if entry.held}<span class="hint">{t.held}</span>{/if}
					</span>
					{#if setup}
						<div class="actions">
							{#if which === 'solos'}
								<label class="pick">
									<input
										type="checkbox"
										checked={picked.includes(entry.id)}
										aria-label={t.pickForPair(names(entry))}
										onchange={() => togglePick(entry)}
									/>
								</label>
							{/if}
							{#if entry.auto_paired}
								<button class="secondary" type="button" disabled={busy} aria-label={t.splitLabel(names(entry))} onclick={() => onsplit(entry)}>
									{t.split}
								</button>
							{/if}
							<button class="danger" type="button" disabled={busy} aria-label={t.removeLabel(names(entry))} onclick={() => onremove(entry)}>
								{t.remove}
							</button>
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
{/snippet}

<style>
	section {
		margin-top: 8px;
	}
	h3 {
		margin: 20px 0 8px;
		font-size: 16px;
		font-weight: 700;
	}
	.layout {
		display: grid;
		gap: 16px 24px;
		align-items: start;
	}
	.layout > * {
		min-width: 0;
	}
	.hint {
		margin: 0 0 8px;
	}
	.note {
		margin: 0 0 12px;
		padding: 12px;
		border: 1px solid var(--warn);
		border-radius: 12px;
		color: var(--warn);
	}
	.link h3 {
		margin-top: 0;
	}
	.url {
		display: block;
		margin-bottom: 8px;
		padding: 10px 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		font-size: 13px;
		overflow-wrap: anywhere;
	}
	.link {
		margin-bottom: 12px;
	}
	.pairing,
	.take {
		margin-top: 16px;
	}
	.pick {
		flex-direction: row;
		align-items: center;
		min-width: var(--tap);
		min-height: var(--tap);
		justify-content: center;
	}
	.pick input {
		width: 24px;
		min-height: 24px;
		padding: 0;
	}
	.danger-zone {
		margin-top: 20px;
	}
	.confirm {
		padding: 12px;
		border: 1px solid var(--warn);
		border-radius: 12px;
	}
	.confirm p {
		margin: 0 0 8px;
	}
	.actions {
		align-items: center;
	}
	@media (min-width: 1100px) {
		.layout {
			grid-template-columns: minmax(0, 1fr) 24rem;
		}
	}
</style>
