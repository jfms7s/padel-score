<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import KindSettings from '#lib/components/events/KindSettings.svelte';
	import { createEvent, eventMessage, listEvents, rulesText } from '#lib/events.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { DECIDING_SETS, DEUCE_RULES, MAX_NAME_LENGTH, SETS_FORMATS } from '#lib/setup.ts';
	import { strings } from '#lib/strings.ts';
	import type { DecidingSet, DeuceRule, EventKind, EventSummary, Format, SetsFormat } from '#lib/types.ts';

	const t = strings.events;
	const KINDS: EventKind[] = ['tournament', 'non_stop'];

	let events = $state<EventSummary[]>([]);
	let loaded = $state(false);
	let loadFailure = $state<string | null>(null);

	let name = $state('');
	let kind = $state<EventKind>('tournament');
	let deuce = $state<DeuceRule>('golden_point');
	let sets = $state<SetsFormat>('best_of_3');
	let deciding = $state<DecidingSet>('super_tie_break');
	let drawsAllowed = $state(false);
	let settings = $state<Record<string, unknown>>({});
	let busy = $state(false);
	let failure = $state<string | null>(null);

	const oneSet = $derived(sets === 'one');
	const format = $derived<Format>(
		kind === 'tournament'
			? { sets: { deuce, sets, deciding: oneSet ? 'full_set' : deciding } }
			: { timed: { deuce, draws_allowed: drawsAllowed } }
	);
	const newestFirst = $derived([...events].sort((x, y) => y.created_at - x.created_at));

	onMount(() => {
		void load();
	});

	async function load() {
		const s = await requireSignedIn();
		if (s.status === 'unavailable') loadFailure = strings.networkError;
		if (s.status !== 'signed_in') return;
		try {
			events = await listEvents();
			loaded = true;
		} catch (e) {
			loadFailure = eventMessage(e);
		}
	}

	function pickKind(k: EventKind) {
		if (k === kind) return;
		kind = k;
		// Each kind has its own settings; don't carry one kind's over to the other.
		settings = {};
	}

	async function create(event: SubmitEvent) {
		event.preventDefault();
		const trimmed = name.trim();
		if (busy || trimmed === '') return;
		busy = true;
		failure = null;
		try {
			const created = await createEvent({ name: trimmed, kind, format, settings });
			await goto(`/events/${encodeURIComponent(created.event.id)}`);
		} catch (e) {
			failure = eventMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct page-wide-desk">
	<a class="back" href="/">{t.back}</a>
	<h1>{t.title}</h1>
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{/if}
	{#if $session.status === 'signed_in' && loaded}
		<p class="hint">{t.intro}</p>

		<div class="split">
		<div class="listcol">
		{#if newestFirst.length === 0}
			<p class="hint">{t.none}</p>
		{:else}
			<ul class="list" aria-label={t.title}>
				{#each newestFirst as ev (ev.id)}
					<li>
						<a class="row event" href="/events/{encodeURIComponent(ev.id)}" aria-label={t.openLabel(ev.name)}>
							<span class="what">
								<strong>{ev.name}</strong>
								<span class="hint">{t.kinds[ev.kind]} · {t.counts(ev.team_count, ev.court_count)}</span>
							</span>
							<span class="badge" class:off={ev.status === 'finished'}>{t.status[ev.status]}</span>
						</a>
					</li>
				{/each}
			</ul>
		{/if}
		</div>

		<div class="formcol">
		<h2>{t.newEvent}</h2>
		<form class="new" onsubmit={create}>
			<label>
				{t.name}
				<input bind:value={name} placeholder={t.namePlaceholder} autocomplete="off" maxlength={MAX_NAME_LENGTH} required />
			</label>

			<div role="group" aria-labelledby="l-kind">
				<span class="label" id="l-kind">{t.kind}</span>
				<div class="seg">
					{#each KINDS as k (k)}
						<button type="button" aria-pressed={kind === k} onclick={() => pickKind(k)}>{t.kinds[k]}</button>
					{/each}
				</div>
			</div>

			<div role="group" aria-labelledby="l-deuce">
				<span class="label" id="l-deuce">{strings.setup.deuce}</span>
				<div class="seg">
					{#each DEUCE_RULES as rule (rule)}
						<button type="button" aria-pressed={deuce === rule} onclick={() => (deuce = rule)}>
							{strings.setup.deuceOptions[rule]}
						</button>
					{/each}
				</div>
			</div>

			{#if kind === 'tournament'}
				<div role="group" aria-labelledby="l-sets">
					<span class="label" id="l-sets">{strings.setup.sets}</span>
					<div class="seg">
						{#each SETS_FORMATS as s (s)}
							<button type="button" aria-pressed={sets === s} onclick={() => (sets = s)}>{strings.setup.setsOptions[s]}</button>
						{/each}
					</div>
				</div>
				<div role="group" aria-labelledby="l-deciding" aria-describedby={oneSet ? 'h-deciding' : undefined}>
					<span class="label" id="l-deciding">{strings.setup.deciding}</span>
					<div class="seg">
						{#each DECIDING_SETS as d (d)}
							<button type="button" disabled={oneSet} aria-pressed={!oneSet && deciding === d} onclick={() => (deciding = d)}>
								{strings.setup.decidingOptions[d]}
							</button>
						{/each}
					</div>
					{#if oneSet}<p class="hint" id="h-deciding">{strings.setup.decidingHint}</p>{/if}
				</div>
			{:else}
				<div role="group" aria-labelledby="l-draws">
					<span class="label" id="l-draws">{t.allowDraws}</span>
					<div class="seg">
						<button type="button" aria-pressed={!drawsAllowed} onclick={() => (drawsAllowed = false)}>{t.drawsOptions.no}</button>
						<button type="button" aria-pressed={drawsAllowed} onclick={() => (drawsAllowed = true)}>{t.drawsOptions.yes}</button>
					</div>
				</div>
			{/if}
			<p class="hint">{rulesText(format)}</p>

			<KindSettings {kind} {settings} onchange={(s) => (settings = s)} />

			{#if failure}<p class="error" role="alert">{failure}</p>{/if}
			<button class="primary" type="submit" disabled={busy}>{busy ? t.creating : t.create}</button>
		</form>
		</div>
		</div>
	{:else if !loadFailure}
		<p class="hint">{strings.auth.loading}</p>
	{/if}
</main>

<style>
	@media (min-width: 1100px) {
		.split {
			display: grid;
			grid-template-columns: minmax(0, 1fr) 28rem;
			gap: 32px;
			align-items: start;
		}
		.formcol h2 {
			margin-top: 0;
		}
	}
	.back {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
	}
	a.event {
		color: inherit;
		text-decoration: none;
		flex-wrap: nowrap;
	}
	a.event strong {
		text-decoration: underline;
		text-decoration-color: var(--line);
		text-underline-offset: 3px;
	}
	.badge {
		flex: none;
	}
	.label {
		display: block;
		font-size: 13px;
		color: var(--muted);
		margin-bottom: 6px;
	}
	.seg {
		display: flex;
		background: var(--panel);
		border: 1px solid var(--line);
		border-radius: 12px;
		padding: 3px;
	}
	.seg button {
		flex: 1;
		min-width: 0;
		font-size: 14px;
		border: 0;
		background: transparent;
		color: var(--ink);
		padding: 8px 4px;
		border-radius: 9px;
		overflow-wrap: anywhere;
	}
	.seg button[aria-pressed='true'] {
		background: var(--ink);
		color: var(--glass);
		font-weight: 650;
	}
	.seg button:disabled {
		cursor: not-allowed;
		opacity: 0.55;
	}
	.new .hint {
		margin: 0;
	}
	p.error {
		margin-bottom: 12px;
	}
</style>
