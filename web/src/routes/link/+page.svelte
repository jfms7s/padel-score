<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError } from '#lib/api.ts';
	import {
		approveDevice,
		authMessage,
		denyDevice,
		formatUserCode,
		initialLinkState,
		isCompleteUserCode,
		lookupDevice,
		type LinkState
	} from '#lib/auth.ts';
	import { requireSignedIn } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';

	const t = strings.auth;
	const l = t.link;

	// Never from the URL (see initialLinkState): the code is typed off the watch.
	let flow = $state<LinkState>(initialLinkState());
	let ready = $state(false);
	let busy = $state(false);
	let failure = $state<string | null>(null);
	let input: HTMLInputElement | undefined = $state();

	onMount(() => {
		void requireSignedIn().then((s) => {
			if (s.status === 'unavailable') failure = strings.networkError;
			ready = s.status === 'signed_in';
		});
	});

	function onInput(event: Event & { currentTarget: HTMLInputElement }) {
		if (flow.step !== 'enter') return;
		const code = formatUserCode(event.currentTarget.value);
		flow = { step: 'enter', code };
		// Keep the field and the formatted value in step, dash and all.
		event.currentTarget.value = code;
	}

	const message = (e: unknown) => (e instanceof ApiError && e.status === 404 ? l.notFound : authMessage(e));

	async function check(event: SubmitEvent) {
		event.preventDefault();
		if (busy || flow.step !== 'enter' || !isCompleteUserCode(flow.code)) return;
		busy = true;
		failure = null;
		const code = flow.code;
		try {
			flow = { step: 'confirm', code, device: await lookupDevice(code) };
		} catch (e) {
			failure = message(e);
		} finally {
			busy = false;
		}
	}

	async function decide(approve: boolean) {
		if (busy || flow.step !== 'confirm') return;
		busy = true;
		failure = null;
		try {
			await (approve ? approveDevice(flow.code) : denyDevice(flow.code));
			flow = { step: 'done', approved: approve };
		} catch (e) {
			failure = message(e);
		} finally {
			busy = false;
		}
	}

	function again() {
		flow = initialLinkState();
		failure = null;
		queueMicrotask(() => input?.focus());
	}
</script>

<main class="acct">
	<h1>{l.title}</h1>
	{#if !ready}
		{#if failure}<p class="error" role="alert">{failure}</p>{:else}<p class="hint">{t.loading}</p>{/if}
	{:else if flow.step === 'enter'}
		<p>{l.intro}</p>
		<form onsubmit={check}>
			<label>
				{l.code}
				<input
					bind:this={input}
					class="code"
					value={flow.code}
					oninput={onInput}
					inputmode="text"
					autocomplete="off"
					autocapitalize="characters"
					spellcheck="false"
					placeholder="XXXX-XXXX"
					maxlength="9"
					required
				/>
			</label>
			{#if failure}<p class="error" role="alert">{failure}</p>{/if}
			<button class="primary" type="submit" disabled={busy || !isCompleteUserCode(flow.code)}>
				{busy ? l.checking : l.next}
			</button>
		</form>
	{:else if flow.step === 'confirm'}
		<div class="device">
			<p class="code-shown">{flow.code}</p>
			<p><strong>{l.device(flow.device.device_name)}</strong></p>
			<p class="hint">{l.made(t.ago(flow.device.created_secs_ago))}</p>
		</div>
		<p class="warning" role="note">{l.warning}</p>
		{#if failure}<p class="error" role="alert">{failure}</p>{/if}
		<div class="actions decide">
			<button class="primary" type="button" disabled={busy} onclick={() => decide(true)}>{l.approve}</button>
			<button class="danger" type="button" disabled={busy} onclick={() => decide(false)}>{l.deny}</button>
		</div>
	{:else}
		<p class="ok" role="status">{flow.approved ? l.approved : l.denied}</p>
		<p><button class="secondary" type="button" onclick={again}>{l.another}</button></p>
	{/if}
</main>

<style>
	.code {
		font-size: 24px;
		font-weight: 750;
		letter-spacing: 0.12em;
		text-transform: uppercase;
		font-feature-settings: 'tnum';
	}
	.device {
		border: 1px solid var(--line);
		border-radius: 16px;
		padding: 14px;
		background: var(--panel);
		margin-bottom: 12px;
	}
	.code-shown {
		font-size: 28px;
		font-weight: 800;
		letter-spacing: 0.12em;
		font-variation-settings: 'wdth' 75;
	}
	.warning {
		border-left: 6px solid var(--ball);
		background: var(--panel);
		border-radius: 12px;
		padding: 12px;
		font-weight: 650;
	}
	.decide button {
		flex: 1;
	}
</style>
