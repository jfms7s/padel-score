<script lang="ts">
	import BackLink from '#lib/components/BackLink.svelte';
	import ChoiceList from '#lib/components/ChoiceList.svelte';
	import { strings } from '#lib/strings.ts';
</script>

<main>
	<BackLink />
	<h1>{strings.create.title}</h1>
	<ChoiceList />
</main>

<style>
	main {
		max-width: 28rem;
		margin: 0 auto;
		padding: 16px 16px 48px;
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
</style>
