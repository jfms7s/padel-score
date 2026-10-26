<!--
	The desk's Teams tab: the teams with rename and (in setup) remove, and the add-team form
	beside them on a desktop; a tournament's settings sit below in setup. The page runs the
	calls and calls `done` once one succeeds, so a form can clear.
-->
<script lang="ts">
	import PlayerPicker from '#lib/components/PlayerPicker.svelte';
	import KindPanel from '#lib/components/events/KindPanel.svelte';
	import { teamText } from '#lib/events.ts';
	import { MAX_NAME_LENGTH } from '#lib/setup.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventDetail, EventTeam, Player } from '#lib/types.ts';

	let {
		detail,
		players,
		playersFailed,
		busy,
		failure,
		onfail,
		onadd,
		onrename,
		onremove,
		onplayer
	}: {
		detail: EventDetail;
		/** The signed-in user's players; null until loaded or if unreadable. */
		players: Player[] | null;
		playersFailed: boolean;
		busy: boolean;
		failure: string | undefined;
		/** Show a message in this tab, or clear the page's failures with null. */
		onfail: (message: string | null) => void;
		onadd: (pair: [string, string], name: string, done: () => void) => void;
		onrename: (team: EventTeam, name: string, done: () => void) => void;
		onremove: (team: EventTeam) => void;
		/** A player made in the picker, for the list. */
		onplayer: (player: Player) => void;
	} = $props();

	const t = strings.events;

	const status = $derived(detail.event.status);
	const teams = $derived(detail.teams);

	let first = $state({ name: '', id: null as string | null });
	let second = $state({ name: '', id: null as string | null });
	let teamName = $state('');
	let renaming = $state<string | null>(null);
	let renameText = $state('');

	const inTeams = $derived(teams.flatMap((tm) => tm.players.map((p) => p.id)));

	function submitTeam(event: SubmitEvent) {
		event.preventDefault();
		if (first.id === null || second.id === null) {
			onfail(t.pickBoth);
			return;
		}
		if (first.id === second.id) {
			onfail(t.samePlayer);
			return;
		}
		onadd([first.id, second.id], teamName.trim(), () => {
			first = { name: '', id: null };
			second = { name: '', id: null };
			teamName = '';
		});
	}

	function startRename(team: EventTeam) {
		renaming = team.id;
		renameText = team.custom_name ?? '';
		onfail(null);
	}

	function submitRename(event: SubmitEvent, team: EventTeam) {
		event.preventDefault();
		onrename(team, renameText.trim(), () => (renaming = null));
	}
</script>

<section aria-labelledby="h-teams">
	<h2 id="h-teams">{t.teams}</h2>
	<div class="layout" class:with-form={status === 'setup'}>
		<div class="list-col">
			{#if teams.length === 0}
				<p class="hint">{t.noTeams}</p>
			{:else}
				<ul class="list grid-auto" style:--min="240px" aria-labelledby="h-teams">
					{#each teams as team (team.id)}
						<li class="row">
							{#if renaming === team.id}
								<form class="inline" onsubmit={(e) => submitRename(e, team)}>
									<!-- svelte-ignore a11y_autofocus -->
									<input
										bind:value={renameText}
										aria-label={t.newTeamName(teamText(team))}
										aria-describedby="h-rename"
										placeholder={strings.teamName(team.players[0].name, team.players[1].name)}
										autocomplete="off"
										maxlength={MAX_NAME_LENGTH}
										autofocus
									/>
									<p class="hint" id="h-rename">{t.newTeamNameHint}</p>
									<div class="actions">
										<button class="primary" type="submit" disabled={busy}>{t.save}</button>
										<button class="secondary" type="button" onclick={() => (renaming = null)}>{t.cancel}</button>
									</div>
								</form>
							{:else}
								<span class="what">
									<strong>{teamText(team)}</strong>
									{#if team.custom_name !== null}
										<span class="hint">{strings.teamName(team.players[0].name, team.players[1].name)}</span>
									{/if}
								</span>
								{#if status !== 'finished'}
									<div class="actions">
										<button class="secondary" type="button" disabled={busy} aria-label={t.renameLabel(teamText(team))} onclick={() => startRename(team)}>
											{t.rename}
										</button>
										{#if status === 'setup'}
											<button class="danger" type="button" disabled={busy} aria-label={t.removeLabel(teamText(team))} onclick={() => onremove(team)}>
												{t.remove}
											</button>
										{/if}
									</div>
								{/if}
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</div>
		{#if status === 'setup'}
			<div class="form-col">
				<h3>{t.addTeam}</h3>
				{#if players}
					<form class="add-team" onsubmit={submitTeam}>
						<div class="slots">
							<div class="slot">
								<PlayerPicker
									bind:value={first.name}
									bind:playerId={first.id}
									{players}
									taken={[...inTeams, ...(second.id ? [second.id] : [])]}
									placeholder={t.teamPlayer(1)}
									label={t.teamPlayer(1)}
									onadd={onplayer}
								/>
							</div>
							<div class="slot">
								<PlayerPicker
									bind:value={second.name}
									bind:playerId={second.id}
									{players}
									taken={[...inTeams, ...(first.id ? [first.id] : [])]}
									placeholder={t.teamPlayer(2)}
									label={t.teamPlayer(2)}
									onadd={onplayer}
								/>
							</div>
						</div>
						{#if players.length === 0}<p class="hint">{t.noPlayers}</p>{/if}
						<label>
							{t.teamName}
							<input bind:value={teamName} autocomplete="off" maxlength={MAX_NAME_LENGTH} />
						</label>
						<button class="primary" type="submit" disabled={busy}>{t.addTeam}</button>
					</form>
				{:else if playersFailed}
					<p class="hint" role="status">{strings.players.listFailed}</p>
				{:else}
					<p class="hint">{strings.auth.loading}</p>
				{/if}
			</div>
		{/if}
	</div>
	{#if failure}<p class="error" role="alert">{failure}</p>{/if}
</section>

{#if status === 'setup' && detail.event.kind === 'tournament'}
	<KindPanel event={detail} audience="coordinator" />
{/if}

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
		gap: 8px 24px;
		align-items: start;
	}
	.layout > * {
		min-width: 0;
	}
	.row .what {
		flex: 1 1 10rem;
	}
	.add-team {
		max-width: 28rem;
	}
	@media (min-width: 1100px) {
		.layout.with-form {
			grid-template-columns: minmax(0, 1fr) 22rem;
		}
		.form-col h3 {
			margin-top: 0;
		}
	}
	.inline {
		flex: 1;
		min-width: 0;
		gap: 8px;
	}
	.inline .hint {
		margin: 0;
	}
	.slots {
		display: grid;
		gap: 8px;
	}
	.slot {
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		padding: 0 10px;
	}
	/* The picker's field sits borderless inside the slot, as in New match. */
	.slot :global(input) {
		border: 0;
		border-radius: 0;
		padding: 2px 0;
		background: transparent;
	}
	p.error {
		margin-top: 12px;
	}
</style>
