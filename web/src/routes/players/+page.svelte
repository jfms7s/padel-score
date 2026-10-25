<script lang="ts">
	import { onMount } from 'svelte';
	import { addPlayer, listPlayers, nameKey, playerMessage, recordText, updatePlayer } from '#lib/players.ts';
	import { MAX_NAME_LENGTH } from '#lib/setup.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { Player, PlayerChange } from '#lib/types.ts';

	const t = strings.players;

	let players = $state<Player[]>([]);
	let loaded = $state(false);
	let loadFailure = $state<string | null>(null);

	let newName = $state('');
	let failure = $state<string | null>(null);
	let done = $state<string | null>(null);
	let busy = $state(false);

	/** The player being renamed, and the name being typed. */
	let editing = $state<string | null>(null);
	let editName = $state('');

	const active = $derived(players.filter((p) => !p.archived));
	const archived = $derived(players.filter((p) => p.archived));

	onMount(() => {
		void load();
	});

	async function load() {
		const s = await requireSignedIn();
		if (s.status === 'unavailable') loadFailure = strings.networkError;
		if (s.status !== 'signed_in') return;
		try {
			players = await listPlayers();
			loaded = true;
		} catch (e) {
			loadFailure = playerMessage(e);
		}
	}

	/** Put the server's answer in the list, in name order as the server keeps it. */
	function put(player: Player) {
		players = [...players.filter((p) => p.id !== player.id), player].sort((x, y) =>
			nameKey(x.name) < nameKey(y.name) ? -1 : nameKey(x.name) > nameKey(y.name) ? 1 : 0
		);
	}

	async function act(run: () => Promise<string | null>) {
		if (busy) return;
		busy = true;
		failure = null;
		done = null;
		try {
			done = await run();
		} catch (e) {
			failure = playerMessage(e);
		} finally {
			busy = false;
		}
	}

	function submitNew(event: SubmitEvent) {
		event.preventDefault();
		const name = newName.trim();
		if (name === '') return;
		void act(async () => {
			const before = players.find((p) => nameKey(p.name) === nameKey(name));
			const player = await addPlayer(name);
			put(player);
			newName = '';
			if (!before) return t.added(player.name);
			return before.archived ? t.restored(player.name) : t.already(player.name);
		});
	}

	function change(player: Player, c: PlayerChange, message: (name: string) => string | null = () => null) {
		void act(async () => {
			const updated = await updatePlayer(player.id, c);
			put(updated);
			return message(updated.name);
		});
	}

	function startRename(player: Player) {
		editing = player.id;
		editName = player.name;
		failure = null;
		done = null;
	}

	function submitRename(event: SubmitEvent, player: Player) {
		event.preventDefault();
		const name = editName.trim();
		if (name === player.name) {
			editing = null;
			return;
		}
		void act(async () => {
			put(await updatePlayer(player.id, { name }));
			editing = null;
			return null;
		});
	}
</script>

<main class="acct">
	<a class="back" href="/">{t.back}</a>
	<h1>{t.title}</h1>
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{/if}
	{#if $session.status === 'signed_in' && loaded}
		<p class="hint">{t.intro}</p>
		<form class="add" onsubmit={submitNew}>
			<input
				bind:value={newName}
				aria-label={t.addLabel}
				placeholder={t.addLabel}
				autocomplete="off"
				maxlength={MAX_NAME_LENGTH}
				required
			/>
			<button class="primary" type="submit" disabled={busy}>{t.add}</button>
		</form>
		{#if failure}<p class="error" role="alert">{failure}</p>{/if}
		{#if done}<p class="ok" role="status">{done}</p>{/if}

		{#if active.length === 0}
			<p class="hint">{t.none}</p>
		{:else}
			<ul class="list" aria-label={t.title}>
				{#each active as player (player.id)}
					<li class="row">
						{#if editing === player.id}
							<form class="rename" onsubmit={(e) => submitRename(e, player)}>
								<!-- svelte-ignore a11y_autofocus -->
								<input
									bind:value={editName}
									aria-label={t.newName(player.name)}
									autocomplete="off"
									maxlength={MAX_NAME_LENGTH}
									required
									autofocus
								/>
								<div class="actions">
									<button class="primary" type="submit" disabled={busy}>{t.save}</button>
									<button class="secondary" type="button" onclick={() => (editing = null)}>{t.cancel}</button>
								</div>
							</form>
						{:else}
							<a class="what" href="/players/{encodeURIComponent(player.id)}">
								<strong>{player.name}</strong>
								<span class="hint">{recordText(player.record)}</span>
							</a>
							<div class="actions">
								<button
									class="secondary"
									type="button"
									disabled={busy}
									aria-label={t.renameLabel(player.name)}
									onclick={() => startRename(player)}>{t.rename}</button
								>
								<button
									class="secondary"
									type="button"
									disabled={busy}
									aria-label={t.archiveLabel(player.name)}
									onclick={() => change(player, { archived: true })}>{t.archive}</button
								>
							</div>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}

		{#if archived.length > 0}
			<h2>{t.archived}</h2>
			<p class="hint">{t.archivedHint}</p>
			<ul class="list" aria-label={t.archived}>
				{#each archived as player (player.id)}
					<li class="row">
						<a class="what" href="/players/{encodeURIComponent(player.id)}">
							<strong>{player.name}</strong>
							<span class="hint">{recordText(player.record)}</span>
						</a>
						<button
							class="secondary"
							type="button"
							disabled={busy}
							aria-label={t.restoreLabel(player.name)}
							onclick={() => change(player, { archived: false }, t.restored)}>{t.restore}</button
						>
					</li>
				{/each}
			</ul>
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
	.add {
		flex-direction: row;
		flex-wrap: wrap;
		gap: 8px;
		margin-bottom: 12px;
	}
	.add input {
		flex: 1 1 12rem;
		width: auto;
	}
	.add button {
		flex: 0 0 auto;
	}
	.list a.what {
		color: inherit;
		text-decoration: none;
		min-height: var(--tap);
		justify-content: center;
		flex: 1 1 10rem;
	}
	.list a.what strong {
		text-decoration: underline;
		text-decoration-color: var(--line);
		text-underline-offset: 3px;
	}
	.rename {
		flex: 1;
		min-width: 0;
		gap: 8px;
	}
	p.ok,
	p.error {
		margin-bottom: 12px;
	}
</style>
