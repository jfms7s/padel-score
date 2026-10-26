<script lang="ts">
	// Two QR codes, "Scoring" (a second phone or a watch can keep score) and "Viewing" (read
	// only), each with a copy-link button.
	import QrCode from '#lib/components/QrCode.svelte';
	import { strings } from '#lib/strings.ts';

	let {
		open = $bindable(false),
		scoring,
		viewing
	}: { open?: boolean; scoring: string; viewing: string | null } = $props();

	const s = strings.share;
	let dialog: HTMLDialogElement | undefined = $state();
	let copied = $state<'scoring' | 'viewing' | null>(null);
	let copyFailed = $state(false);

	$effect(() => {
		if (!dialog) return;
		if (open && !dialog.open) dialog.showModal();
		if (!open && dialog.open) dialog.close();
	});

	async function copy(which: 'scoring' | 'viewing', link: string) {
		copyFailed = false;
		try {
			await navigator.clipboard.writeText(link);
			copied = which;
			setTimeout(() => {
				if (copied === which) copied = null;
			}, 2000);
		} catch {
			copyFailed = true;
		}
	}
</script>

<dialog bind:this={dialog} onclose={() => (open = false)} aria-labelledby="share-title">
	<div class="sheet">
		<header>
			<h2 id="share-title">{s.title}</h2>
			<form method="dialog">
				<button class="close" type="submit">{s.close}</button>
			</form>
		</header>
		<div class="codes">
			{#each [{ key: 'scoring', label: s.scoring, hint: s.scoringHint, link: scoring }, { key: 'viewing', label: s.viewing, hint: s.viewingHint, link: viewing }] as item (item.key)}
				{@const key = item.key as 'scoring' | 'viewing'}
				<figure class="qr">
					{#if item.link}
						<div class="code"><QrCode text={item.link} label={s.qr(item.label)} /></div>
						<figcaption>
							<strong>{item.label}</strong>
							<span class="hint">{item.hint}</span>
							<input class="link" readonly value={item.link} aria-label={item.label} onclick={(e) => e.currentTarget.select()} />
							<button type="button" class="copy" onclick={() => copy(key, item.link ?? '')}>
								{copied === key ? s.copied : s.copy}
							</button>
						</figcaption>
					{:else}
						<figcaption>
							<strong>{item.label}</strong>
							<span class="hint">{s.noViewing}</span>
						</figcaption>
					{/if}
				</figure>
			{/each}
		</div>
		{#if copyFailed}<p class="hint" role="alert">{s.copyFailed}</p>{/if}
	</div>
</dialog>

<style>
	dialog {
		width: min(100vw - 32px, 40rem);
		max-height: calc(100dvh - 32px);
		padding: 0;
		border: 1px solid var(--line);
		border-radius: 18px;
		background: var(--panel);
		color: var(--ink);
	}
	dialog::backdrop {
		background: rgba(5, 11, 18, 0.55);
	}
	.sheet {
		padding: 20px 18px;
	}
	header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 12px;
		margin-bottom: 14px;
	}
	h2 {
		margin: 0;
		font-size: 22px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	.codes {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
		gap: 16px;
	}
	.qr {
		margin: 0;
		display: flex;
		flex-direction: column;
		gap: 10px;
		border: 1px solid var(--line);
		border-radius: 16px;
		padding: 14px;
		background: var(--glass);
	}
	.code {
		display: flex;
		justify-content: center;
	}
	figcaption {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.hint {
		font-size: 13px;
		color: var(--muted);
	}
	.link {
		min-height: var(--tap);
		width: 100%;
		font-size: 13px;
		padding: 0 10px;
		border: 1px solid var(--line);
		border-radius: 10px;
		background: var(--panel);
	}
	.copy,
	.close {
		border-radius: 12px;
		font-weight: 650;
		padding: 0 16px;
	}
	.copy {
		border: 0;
		background: var(--ink);
		color: var(--glass);
	}
	.close {
		background: transparent;
		color: var(--ink);
		border: 1px solid var(--line);
	}
</style>
