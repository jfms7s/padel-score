<script lang="ts">
	// Joining a board signed out: type a name, or take the name of a guest who is already here
	// (a new browser, or cleared site data).
	import { guestsToPick, submitGuestName, submitPick, type JoinResult } from '#lib/guestjoin.ts';
	import { MAX_NAME_LENGTH } from '#lib/setup.ts';
	import { strings } from '#lib/strings.ts';
	import type { BoardView } from '#lib/types.ts';

	const t = strings.boards;

	interface Props {
		view: BoardView;
		token: string;
		onjoined: (v: BoardView) => void;
	}

	let { view, token, onjoined }: Props = $props();

	let name = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);
	const guests = $derived(guestsToPick(view));

	async function run(call: () => Promise<JoinResult>) {
		error = null;
		busy = true;
		const result = await call();
		busy = false;
		if ('error' in result) error = result.error;
		else onjoined(result.view);
	}

	function join(event: SubmitEvent) {
		event.preventDefault();
		if (name.trim() === '') return;
		void run(() => submitGuestName(token, name));
	}
</script>

<div class="guestjoin">
	<form onsubmit={join}>
		<label>
			{t.guestName}
			<input bind:value={name} autocomplete="off" maxlength={MAX_NAME_LENGTH} required />
		</label>
		<button type="submit" class="secondary" disabled={busy}>{t.joinAsGuest}</button>
	</form>
	{#if guests.length > 0}
		<details>
			<summary>{t.alreadyHere}</summary>
			<h3>{t.pickYourName}</h3>
			<ul class="list">
				{#each guests as g (g.user_id)}
					<li>
						<button type="button" class="row" disabled={busy} onclick={() => run(() => submitPick(token, g.user_id))}>
							{g.name}
						</button>
					</li>
				{/each}
			</ul>
		</details>
	{/if}
	{#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
	.guestjoin {
		display: grid;
		gap: 8px;
		flex-basis: 100%;
	}
	form {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-end;
		gap: 8px 12px;
	}
	summary {
		min-height: var(--tap);
		display: flex;
		align-items: center;
		cursor: pointer;
		color: var(--muted);
	}
	h3 {
		margin: 4px 0;
		font-size: 14px;
		color: var(--muted);
	}
</style>
