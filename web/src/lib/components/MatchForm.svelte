<script lang="ts">
	import BackLink from '#lib/components/BackLink.svelte';
	import { untrack } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { createMatch } from '#lib/api.ts';
	import PlayerPicker from '#lib/components/PlayerPicker.svelte';
	import PlayLaterFields from '#lib/components/PlayLaterFields.svelte';
	import ServeOrderFields from '#lib/components/ServeOrderFields.svelte';
	import { startFailure, startFromParam } from '#lib/handoff.ts';
	import { listPlayers } from '#lib/players.ts';
	import { createScheduled, emptyLater, laterStart, scheduledMessage, toNewScheduled } from '#lib/scheduled.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { safeStorage } from '#lib/storage.ts';
	import { strings } from '#lib/strings.ts';
	import {
		DECIDING_SETS,
		DEUCE_RULES,
		SETS_FORMATS,
		afterCreate,
		MAX_NAME_LENGTH,
		defaultSetup,
		loadLastSetup,
		setSets,
		toCreateRequest
	} from '#lib/setup.ts';
	import type { Setup } from '#lib/setup.ts';
	import type { Player, StartedFrom } from '#lib/types.ts';

	let { mode }: { mode: 'now' | 'later' } = $props();
	// Each route mounts its own form, so the mode never changes while it lives.
	// svelte-ignore state_referenced_locally
	const isLater = mode === 'later';
	const t = strings.setup;
	const TEAMS = ['a', 'b'] as const;
	const slots = [
		{ team: 'a', index: 0, n: 1, label: t.teamPlayer('A', 1) },
		{ team: 'a', index: 1, n: 2, label: t.teamPlayer('A', 2) },
		{ team: 'b', index: 0, n: 3, label: t.teamPlayer('B', 1) },
		{ team: 'b', index: 1, n: 4, label: t.teamPlayer('B', 2) }
	] as const;

	/** Plan a match: the places are picked players or open, so typed names are dropped. */
	function forLater(start: Setup): Setup {
		for (const team of TEAMS) {
			for (const index of [0, 1] as const) {
				if (start.players[team][index] === null) start.teams[team][index] = '';
			}
		}
		return start;
	}

	let setup = $state(isLater ? forLater(loadLastSetup(safeStorage())) : defaultSetup());
	/** Plan a match: the match is saved for a day and time instead of started. */
	let later = $state(isLater ? { ...emptyLater(), on: true } : emptyLater());
	/** What the match starts from (a game), once its hand-off was found; sent with the create. */
	let from = $state<StartedFrom | null>(null);
	/** `?from=` named a source but its hand-off is gone (a reload, another tab). */
	let handoffExpired = $state(false);
	// The `from` value alone: the effect below runs again only when it changes, not when
	// something else on the page URL does or after the hand-off was taken.
	const fromRaw = $derived(page.url.searchParams.get('from'));
	$effect(() => {
		if (isLater) return;
		const raw = fromRaw;
		// The hand-off is taken once per `from`, so a reload falls back to a plain New match.
		untrack(() => {
			const started = startFromParam(raw, loadLastSetup(safeStorage()));
			setup = started.setup;
			from = started.from;
			handoffExpired = started.expired;
		});
	});

	// Starting a match needs an account: signed out, the browser goes to sign-in and back here.
	/** The signed-in user's players; null while unknown or unreadable (plain inputs). */
	let players = $state<Player[] | null>(null);
	let playersFailed = $state(false);
	let unavailable = $state(false);
	$effect(() => {
		void loadPlayers();
	});

	async function loadPlayers() {
		const s = await requireSignedIn();
		if (s.status === 'unavailable') unavailable = true;
		if (s.status !== 'signed_in') return;
		try {
			players = await listPlayers({ quiet: true });
		} catch {
			playersFailed = true;
		}
	}

	/** The players picked in every slot but this one. */
	function takenBesides(team: 'a' | 'b', index: number): string[] {
		return slots
			.filter((s) => s.team !== team || s.index !== index)
			.map((s) => setup.players[s.team][s.index])
			.filter((id): id is string => id !== null);
	}

	function added(player: Player) {
		if (!players) return;
		players = [...players.filter((p) => p.id !== player.id), player];
	}

	let busy = $state(false);
	let failure = $state<string | null>(null);
	let failureLead = $state<string>(t.failed);
	let stranded = $state<string | null>(null);

	const oneSet = $derived(setup.sets === 'one');

	// Pressed by the user, once per press: createMatch has no idempotency key, so it is never
	// retried automatically (a lost reply would orphan a match).
	async function start(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		if (later.on) return void schedule();
		busy = true;
		failure = null;
		failureLead = t.failed;
		let created;
		const req = toCreateRequest(setup, from ?? undefined);
		try {
			created = await createMatch(req);
		} catch (e) {
			const refused = startFailure(e, from !== null, from?.kind);
			if (refused.dropFrom) {
				// The game can't start this match any more: a retry makes a plain, unlinked one.
				from = null;
			}
			failure =
				refused.message !== null
					? refused.dropFrom
						? `${refused.message} ${t.unlinked}`
						: refused.message
					: e instanceof Error
						? e.message
						: '';
			busy = false;
			return;
		}
		// The match exists now: nothing below may report a failure or invite a second create.
		const done = await afterCreate({
			storage: safeStorage(),
			setup,
			scorerToken: created.scorer_token,
			spectatorToken: created.spectator_token,
			teams: req.teams,
			now: Date.now(),
			go: (path) => goto(path)
		});
		if (!done.navigated) stranded = done.path;
		busy = false;
	}

	/** Plan a match: save it and open its page, which has the link to share. */
	async function schedule() {
		busy = true;
		failure = null;
		failureLead = t.scheduleFailed;
		const startsAt = laterStart(later, Math.floor(Date.now() / 1000));
		if (startsAt === null) {
			failure = strings.scheduled.needsStart;
			busy = false;
			return;
		}
		let made;
		try {
			made = await createScheduled(toNewScheduled(setup, { startsAt, place: later.place, listing: later.listing }));
		} catch (e) {
			failure = scheduledMessage(e);
			busy = false;
			return;
		}
		// The match exists now: a failed navigation must not read as a failed save (a retry would duplicate it).
		const path = `/j/${encodeURIComponent(made.join_token)}`;
		try {
			await goto(path);
		} catch {
			stranded = path;
		}
		busy = false;
	}
</script>

<main>
	<BackLink />
	{#if unavailable}
		<p class="error" role="alert">{strings.networkError}</p>
	{/if}
	{#if $session.status === 'signed_in'}
	<form onsubmit={start}>
		<h1>{isLater ? strings.create.choices.later[0] : t.title}</h1>
		{#if handoffExpired}<p class="hint" role="status">{t.handoffExpired}</p>{/if}

		<div class="cols">
		<div class="col">
		<div class="field" role="group" aria-labelledby="l-deuce">
			<span class="label" id="l-deuce">{t.deuce}</span>
			<div class="seg">
				{#each DEUCE_RULES as rule (rule)}
					<button type="button" aria-pressed={setup.deuce === rule} onclick={() => (setup.deuce = rule)}>
						{t.deuceOptions[rule]}
					</button>
				{/each}
			</div>
		</div>

		<div class="field" role="group" aria-labelledby="l-sets">
			<span class="label" id="l-sets">{t.sets}</span>
			<div class="seg">
				{#each SETS_FORMATS as sets (sets)}
					<button type="button" aria-pressed={setup.sets === sets} onclick={() => (setup = setSets(setup, sets))}>
						{t.setsOptions[sets]}
					</button>
				{/each}
			</div>
		</div>

		<div class="field" role="group" aria-labelledby="l-deciding" aria-describedby={oneSet ? 'h-deciding' : undefined}>
			<span class="label" id="l-deciding">{t.deciding}</span>
			<div class="seg">
				{#each DECIDING_SETS as deciding (deciding)}
					<button
						type="button"
						disabled={oneSet}
						aria-pressed={setup.deciding === deciding}
						onclick={() => (setup.deciding = deciding)}
					>
						{t.decidingOptions[deciding]}
					</button>
				{/each}
			</div>
			{#if oneSet}<p class="hint" id="h-deciding">{t.decidingHint}</p>{/if}
		</div>

		</div>
		<div class="col">
		{#if later.on}<PlayLaterFields bind:draft={later} />{/if}

		<div class="field" role="group" aria-labelledby="l-teams" aria-describedby={later.on ? 'h-open' : undefined}>
			<span class="label" id="l-teams">{t.teams}</span>
			{#if later.on}<p class="hint top" id="h-open">{t.openSlots}</p>{/if}
			<div class="teams">
				{#each TEAMS as team (team)}
					<div class="team-in {team}">
						{#each slots.filter((s) => s.team === team) as slot (slot.n)}
							<div class="slot">
								{#if players}
									<PlayerPicker
										bind:value={setup.teams[team][slot.index]}
										bind:playerId={setup.players[team][slot.index]}
										{players}
										taken={takenBesides(team, slot.index)}
										placeholder={later.on ? t.open : t.player(slot.n)}
										label={slot.label}
										pickOnly={later.on}
										onadd={added}
									/>
								{:else}
									<input bind:value={setup.teams[team][slot.index]} placeholder={t.player(slot.n)} aria-label={slot.label} autocomplete="off" maxlength={MAX_NAME_LENGTH} />
								{/if}
							</div>
						{/each}
					</div>
				{/each}
			</div>
			{#if playersFailed}<p class="hint" role="status">{strings.players.listFailed}</p>
			{:else if later.on && players === null}<p class="hint" role="status">{strings.auth.loading}</p>{/if}
		</div>

		{#if later.on}
			<p class="hint">{t.serveLater}</p>
		{:else}
			<ServeOrderFields bind:order={setup.serve} teams={setup.teams} />
		{/if}

		{#if stranded}
			<p class="ok" role="status">{t.created} <a href={stranded}>{t.openMatch}</a></p>
		{/if}

		{#if failure}
			<div class="error" role="alert">
				<p>{failure ? `${failureLead} ${failure}` : failureLead}</p>
				<button type="submit" class="retry" disabled={busy}>{t.tryAgain}</button>
			</div>
		{/if}

		<button class="primary" type="submit" disabled={busy || (later.on && players === null)}>{busy ? (later.on ? t.saving : t.starting) : later.on ? t.saveMatch : t.start}</button>
		</div>
		</div>
	</form>
	{/if}
</main>

<style>
	main {
		max-width: 28rem;
		margin: 0 auto;
		padding: 16px 16px 48px;
	}
	.cols,
	.col {
		display: flex;
		flex-direction: column;
		gap: 16px;
	}
	@media (min-width: 1100px) {
		main {
			max-width: 60rem;
		}
		.cols {
			display: grid;
			grid-template-columns: repeat(2, minmax(0, 1fr));
			gap: 32px;
			align-items: start;
		}
	}
	form {
		display: flex;
		flex-direction: column;
		gap: 16px;
	}
	h1 {
		margin: 0;
		font-size: 22px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
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
	.teams {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 8px;
	}
	.team-in {
		border-radius: 12px;
		padding: 6px 10px;
		background: var(--panel);
		border: 1px solid var(--line);
	}
	.team-in.a {
		border-left: 6px solid var(--a);
	}
	.team-in.b {
		border-left: 6px solid var(--b);
	}
	.team-in input {
		width: 100%;
		min-height: var(--tap);
		border: 0;
		background: transparent;
		padding: 2px 0;
	}
	.hint.top {
		margin: 0 0 8px;
	}
	.slot + .slot {
		border-top: 1px solid var(--line);
	}
	.hint {
		font-size: 12px;
		color: var(--muted);
		margin: 6px 0 0;
	}
	.primary {
		min-height: var(--tap);
		border: 0;
		border-radius: 14px;
		background: var(--ink);
		color: var(--glass);
		font-size: 17px;
		font-weight: 650;
		padding: 14px;
		margin-top: 8px;
	}
	.primary:disabled {
		opacity: 0.6;
	}
	.error {
		border: 1px solid var(--warn);
		border-radius: 12px;
		padding: 12px;
		color: var(--warn);
	}
	.error p {
		margin: 0 0 8px;
	}
	.ok a {
		color: var(--ink);
		font-weight: 650;
	}
	.retry {
		border: 1px solid var(--warn);
		background: transparent;
		color: var(--warn);
		border-radius: 10px;
		padding: 8px 16px;
		font-weight: 650;
	}
</style>
