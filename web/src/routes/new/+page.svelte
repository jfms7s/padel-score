<script lang="ts">
	import { goto } from '$app/navigation';
	import { createMatch } from '#lib/api.ts';
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
		serveChoices,
		setSets,
		toCreateRequest
	} from '#lib/setup.ts';

	const t = strings.setup;

	let setup = $state(defaultSetup());
	$effect(() => {
		setup = loadLastSetup(safeStorage());
	});

	let busy = $state(false);
	let failure = $state<string | null>(null);
	let stranded = $state<string | null>(null);

	const oneSet = $derived(setup.sets === 'one');
	const choices = $derived(serveChoices(setup));

	function pickFirstServer(team: 'a' | 'b', index: number) {
		setup.firstServer = { team, index };
	}

	// Pressed by the user, once per press: createMatch has no idempotency key, so it is never
	// retried automatically (a lost reply would orphan a match).
	async function start(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		busy = true;
		failure = null;
		let created;
		const req = toCreateRequest(setup);
		try {
			created = await createMatch(req);
		} catch (e) {
			failure = e instanceof Error ? e.message : '';
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
</script>

<main>
	<a class="back" href="/">{t.back}</a>
	<form onsubmit={start}>
		<h1>{t.title}</h1>

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

		<div class="field" role="group" aria-labelledby="l-teams">
			<span class="label" id="l-teams">{t.teams}</span>
			<div class="teams">
				<div class="team-in a">
					<input bind:value={setup.teams.a[0]} placeholder={t.player(1)} aria-label={t.teamPlayer('A', 1)} autocomplete="off" maxlength={MAX_NAME_LENGTH} />
					<input bind:value={setup.teams.a[1]} placeholder={t.player(2)} aria-label={t.teamPlayer('A', 2)} autocomplete="off" maxlength={MAX_NAME_LENGTH} />
				</div>
				<div class="team-in b">
					<input bind:value={setup.teams.b[0]} placeholder={t.player(3)} aria-label={t.teamPlayer('B', 1)} autocomplete="off" maxlength={MAX_NAME_LENGTH} />
					<input bind:value={setup.teams.b[1]} placeholder={t.player(4)} aria-label={t.teamPlayer('B', 2)} autocomplete="off" maxlength={MAX_NAME_LENGTH} />
				</div>
			</div>
		</div>

		<div class="field" role="group" aria-labelledby="l-first">
			<span class="label" id="l-first">{t.firstServer}</span>
			<div class="seg">
				{#each choices.first as c (`${c.player.team}${c.player.index}`)}
					<button
						type="button"
						aria-pressed={setup.firstServer.team === c.player.team && setup.firstServer.index === c.player.index}
						onclick={() => pickFirstServer(c.player.team, c.player.index)}
					>
						{c.label}
					</button>
				{/each}
			</div>
			<p class="hint">{t.firstServerHint}</p>
		</div>

		<div class="field" role="group" aria-labelledby="l-other">
			<span class="label" id="l-other">{t.otherFirstServer}</span>
			<div class="seg">
				{#each choices.other as c (c.index)}
					<button type="button" aria-pressed={setup.otherFirstServer === c.index} onclick={() => (setup.otherFirstServer = c.index)}>
						{c.label}
					</button>
				{/each}
			</div>
		</div>

		{#if stranded}
			<p class="ok" role="status">{t.created} <a href={stranded}>{t.openMatch}</a></p>
		{/if}

		{#if failure}
			<div class="error" role="alert">
				<p>{failure ? `${t.failed} ${failure}` : t.failed}</p>
				<button type="submit" class="retry" disabled={busy}>{t.tryAgain}</button>
			</div>
		{/if}

		<button class="primary" type="submit" disabled={busy}>{busy ? t.starting : t.start}</button>
	</form>
</main>

<style>
	main {
		max-width: 28rem;
		margin: 0 auto;
		padding: 16px 16px 48px;
	}
	.back {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
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
	.team-in input + input {
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
